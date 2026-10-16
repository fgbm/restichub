//! The query values that the filtered lists share (FR-UI-01): comma lists, the sort order and the
//! page window.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::validation::Reason;

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

/// The values of a comma list without blanks. A value never holds a comma.
pub fn split_list(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Parses each value of a comma list; an unknown value gives HTTP 400 for the field.
pub fn parse_list<T>(
    field: &str,
    value: Option<&str>,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Vec<T>, ApiError> {
    split_list(value)
        .iter()
        .map(|item| parse(item).ok_or_else(|| ApiError::invalid_field(field, Reason::Unknown)))
        .collect()
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

/// A page reads either from a cursor or from an offset, and a cursor follows the default order.
pub fn check_window(
    cursor: Option<&str>,
    offset: Option<i64>,
    default_order: bool,
) -> Result<(), ApiError> {
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
    use super::{check_window, parse_list, split_list};

    #[test]
    fn a_comma_list_drops_the_blanks() {
        assert_eq!(split_list(Some(" linux, ,windows ")), ["linux", "windows"]);
        assert!(split_list(None).is_empty());
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
    }
}
