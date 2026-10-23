//! The query values that the filtered lists share (FR-UI-01): comma lists, the sort order and the
//! page window.

// The raw extractor is the only place that parses a query: this wrapper maps its rejection.
#[allow(clippy::disallowed_types)]
use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;

use restichub_db::job_search::CappedCount;

use crate::error::ApiError;
use crate::validation::Reason;

/// Limit in characters.
pub const MAX_SEARCH: usize = 200;
pub const MAX_LIST_VALUES: usize = 50;
/// A page past the counted rows has no total to show; a narrower filter reaches those rows.
pub const MAX_OFFSET: i64 = restichub_db::job_search::COUNT_LIMIT;

/// Query parameters with the problem-response mapping of the control plane.
///
/// A value that does not parse gives HTTP 400 `invalid_argument` that names the parameter, so the
/// client gets a stable code also for an enum parameter.
#[derive(Debug)]
pub struct ApiQuery<T>(pub T);

#[allow(clippy::disallowed_types)]
impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::try_from_uri(&parts.uri) {
            Ok(Query(value)) => Ok(Self(value)),
            Err(rejection) => Err(query_error(&rejection.body_text())),
        }
    }
}

const QUERY_ERROR_PREFIX: &str = "Failed to deserialize query string: ";

/// The message holds the `serde_path_to_error` path, which for a flat query is the parameter
/// name. An unknown parameter or any path that is not a plain name falls back to `query`, so the
/// problem body never echoes the request.
fn query_error(message: &str) -> ApiError {
    let rest = message.strip_prefix(QUERY_ERROR_PREFIX).unwrap_or_default();
    let (path, detail) = rest.split_once(": ").unwrap_or(("", rest));
    if detail.starts_with("unknown field") {
        return ApiError::invalid_field("query", Reason::Unknown);
    }
    let named = !path.is_empty()
        && path.len() <= 32
        && path
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_');
    ApiError::invalid_field(if named { path } else { "query" }, Reason::InvalidFormat)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

impl SortOrder {
    pub fn descending(self) -> bool {
        self == Self::Desc
    }
}

/// One value of a filter with the number of rows that hold it.
#[derive(Debug, Serialize, ToSchema)]
pub struct FacetValue {
    pub value: String,
    pub count: i64,
}

impl From<(String, i64)> for FacetValue {
    fn from((value, count): (String, i64)) -> Self {
        Self { value, count }
    }
}

/// The number of rows of a state tab under the other filters of the list.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub struct TabCount {
    /// At most 10000.
    pub count: i64,
    /// More rows match than `count` shows.
    pub capped: bool,
}

impl From<CappedCount> for TabCount {
    fn from(value: CappedCount) -> Self {
        Self {
            count: value.count,
            capped: value.capped,
        }
    }
}

/// The values of a comma list without blanks. A value never holds a comma.
pub fn split_list(field: &str, value: Option<&str>) -> Result<Vec<String>, ApiError> {
    let items: Vec<String> = value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect();
    if items.len() > MAX_LIST_VALUES {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    Ok(items)
}

/// Parses each value of a comma list; an unknown value gives HTTP 400 for the field.
pub fn parse_list<T>(
    field: &str,
    value: Option<&str>,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Vec<T>, ApiError> {
    split_list(field, value)?
        .iter()
        .map(|item| parse(item).ok_or_else(|| ApiError::invalid_field(field, Reason::Unknown)))
        .collect()
}

/// The search text without the outer blanks; a blank text does not narrow the list.
pub fn parse_search(value: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(text) = value.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if text.chars().count() > MAX_SEARCH {
        return Err(ApiError::invalid_field("q", Reason::TooLong));
    }
    Ok(Some(text.to_owned()))
}

pub fn parse_time(field: &str, value: Option<&str>) -> Result<Option<OffsetDateTime>, ApiError> {
    value
        .map(|text| {
            OffsetDateTime::parse(text, &Rfc3339).map_err(|cause| {
                ApiError::invalid_field_cause(field, Reason::InvalidFormat, cause.to_string())
            })
        })
        .transpose()
}

pub fn check_period(
    from: Option<OffsetDateTime>,
    to: Option<OffsetDateTime>,
) -> Result<(), ApiError> {
    match (from, to) {
        (Some(from), Some(to)) if from > to => {
            Err(ApiError::invalid_field("to", Reason::OutOfRange))
        }
        _ => Ok(()),
    }
}

/// A page reads either from a cursor or from an offset in `0..=MAX_OFFSET`, and a cursor follows
/// the default order.
pub fn check_window(
    cursor: Option<&str>,
    offset: Option<i64>,
    default_order: bool,
) -> Result<(), ApiError> {
    if offset.is_some_and(|offset| !(0..=MAX_OFFSET).contains(&offset)) {
        return Err(ApiError::invalid_field("offset", Reason::OutOfRange));
    }
    if cursor.is_none() {
        return Ok(());
    }
    if offset.is_some() {
        return Err(ApiError::invalid_field_cause(
            "offset",
            Reason::InvalidFormat,
            "a page reads either a cursor or an offset",
        ));
    }
    if !default_order {
        return Err(ApiError::invalid_field_cause(
            "sort",
            Reason::InvalidFormat,
            "a cursor follows the default order",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_LIST_VALUES, MAX_OFFSET, MAX_SEARCH, check_window, parse_list, parse_search, split_list,
    };

    #[test]
    fn a_comma_list_drops_the_blanks() {
        assert_eq!(
            split_list("x", Some(" linux, ,windows ")).ok(),
            Some(vec!["linux".to_owned(), "windows".to_owned()])
        );
        assert_eq!(split_list("x", None).ok(), Some(Vec::new()));
    }

    #[test]
    fn a_long_list_and_a_long_search_are_refused() {
        let list = vec!["a"; MAX_LIST_VALUES + 1].join(",");
        assert!(split_list("x", Some(&list)).is_err());
        assert!(split_list("x", Some(&list[2..])).is_ok());
        assert!(parse_search(Some(&"ж".repeat(MAX_SEARCH))).is_ok());
        assert!(parse_search(Some(&"ж".repeat(MAX_SEARCH + 1))).is_err());
        assert_eq!(parse_search(Some("  ")).ok(), Some(None));
    }

    #[test]
    fn an_unknown_value_of_a_list_is_refused() {
        let parse = |value: &str| (value == "a").then_some(1);
        assert_eq!(parse_list("x", Some("a,a"), parse).ok(), Some(vec![1, 1]));
        assert!(parse_list("x", Some("a,b"), parse).is_err());
    }

    #[test]
    fn a_cursor_takes_no_offset_and_no_other_order() {
        assert!(check_window(Some("c"), None, true).is_ok());
        assert!(check_window(Some("c"), Some(0), true).is_err());
        assert!(check_window(Some("c"), None, false).is_err());
        assert!(check_window(None, Some(20), false).is_ok());
        assert!(check_window(None, Some(MAX_OFFSET), false).is_ok());
        assert!(check_window(None, Some(MAX_OFFSET + 1), false).is_err());
        assert!(check_window(None, Some(-1), false).is_err());
    }
}
