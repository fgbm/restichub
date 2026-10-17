//! The identity of a repository location (FR-RP-11).
//!
//! Two repositories of one tenant cannot hold the same location: two writers on one restic
//! repository share its locks and its index, and a delete of one hides the other. The key
//! below folds the spellings that name the same place.

use restichub_db::TenantTx;
use restichub_db::repositories::{self as repo, Repository};
use restichub_db::{AgentId, BackendKind, RepositoryId};

use super::settings::ValidatedBackend;
use crate::error::ApiError;

/// The scope of the advisory lock that serializes the location check with the write.
pub(super) const LOCATION_LOCK: &str = "repository-location";

/// Folds the scheme, the host and the default port of an endpoint. The path stays as it is.
fn endpoint_key(endpoint: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    let Some((scheme, rest)) = endpoint.split_once("://") else {
        return endpoint.to_lowercase();
    };
    let scheme = scheme.to_lowercase();
    let (authority, path) = match rest.find('/') {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let authority = authority.to_lowercase();
    let default_port = match scheme.as_str() {
        "https" => ":443",
        "http" => ":80",
        _ => "",
    };
    let authority = authority.strip_suffix(default_port).unwrap_or(&authority);
    format!("{scheme}://{authority}{}", path.trim_end_matches('/'))
}

/// A Windows path has no case and takes both separators; a POSIX path keeps its case.
fn path_key(path: &str) -> String {
    let windows = path.as_bytes().get(1) == Some(&b':');
    let separators: &[char] = if windows { &['/', '\\'] } else { &['/'] };
    let joined = path
        .split(separators)
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>()
        .join("/");
    if windows {
        joined.to_lowercase()
    } else {
        format!("/{joined}")
    }
}

fn s3_key(endpoint: &str, bucket: &str, prefix: Option<&str>) -> String {
    let prefix = prefix.map(|value| value.trim_matches('/')).unwrap_or("");
    format!("s3|{}|{bucket}|{prefix}", endpoint_key(endpoint))
}

/// The key of a location that passed the validation.
pub(super) fn key_of_backend(backend: &ValidatedBackend, host: Option<AgentId>) -> String {
    match backend.kind {
        BackendKind::S3 => s3_key(
            backend.s3_endpoint.as_deref().unwrap_or_default(),
            backend.s3_bucket.as_deref().unwrap_or_default(),
            backend.s3_prefix.as_deref(),
        ),
        BackendKind::Rest => format!(
            "rest|{}",
            endpoint_key(backend.rest_endpoint.as_deref().unwrap_or_default())
        ),
        BackendKind::Local => local_key(backend.local_path.as_deref().unwrap_or_default(), host),
    }
}

pub(super) fn key_of_repository(repository: &Repository) -> String {
    match repository.backend {
        BackendKind::S3 => s3_key(
            repository.s3_endpoint.as_deref().unwrap_or_default(),
            repository.s3_bucket.as_deref().unwrap_or_default(),
            repository.s3_prefix.as_deref(),
        ),
        BackendKind::Rest => format!(
            "rest|{}",
            endpoint_key(repository.rest_endpoint.as_deref().unwrap_or_default())
        ),
        BackendKind::Local => local_key(
            repository.local_path.as_deref().unwrap_or_default(),
            repository.maintenance_agent_id,
        ),
    }
}

fn local_key(path: &str, host: Option<AgentId>) -> String {
    let host = host.map(|id| id.to_string()).unwrap_or_default();
    format!("local|{host}|{}", path_key(path))
}

/// The input of the form that shows the error of a taken location.
pub(super) fn location_field(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::S3 => "backend.bucket",
        BackendKind::Rest => "backend.endpoint",
        BackendKind::Local => "backend.path",
    }
}

/// Refuses a location that another repository of the tenant holds (FR-RP-11).
///
/// The lock serializes two concurrent writes of one location: the second waits, then sees the
/// first row.
pub(super) async fn ensure_location_free(
    tx: &mut TenantTx<'_>,
    key: &str,
    kind: BackendKind,
    except: Option<RepositoryId>,
) -> Result<(), ApiError> {
    tx.lock(LOCATION_LOCK).await?;
    let repositories = repo::list(tx).await?;
    match holder(&repositories, key, except) {
        Some(existing) => Err(ApiError::repository_location_taken(
            location_field(kind),
            existing.name.clone(),
        )),
        None => Ok(()),
    }
}

/// The oldest other repository that holds the key.
pub(super) fn holder<'a>(
    repositories: &'a [Repository],
    key: &str,
    except: Option<RepositoryId>,
) -> Option<&'a Repository> {
    repositories
        .iter()
        .filter(|repository| Some(repository.id) != except && key_of_repository(repository) == key)
        .min_by_key(|repository| repository.created_at)
}

/// The names of the other repositories that hold the same location, for each repository.
pub(crate) fn sharing_names(repositories: &[Repository]) -> Vec<Vec<String>> {
    let keys: Vec<String> = repositories.iter().map(key_of_repository).collect();
    keys.iter()
        .enumerate()
        .map(|(at, key)| {
            repositories
                .iter()
                .zip(&keys)
                .enumerate()
                .filter(|(other, (_, other_key))| *other != at && *other_key == key)
                .map(|(_, (repository, _))| repository.name.clone())
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{endpoint_key, path_key};

    #[test]
    fn an_endpoint_folds_case_port_and_trailing_slash() {
        assert_eq!(
            endpoint_key("HTTPS://S3.Example.COM:443/"),
            endpoint_key("https://s3.example.com")
        );
        assert_ne!(
            endpoint_key("https://s3.example.com/Path"),
            endpoint_key("https://s3.example.com/path")
        );
        assert_ne!(
            endpoint_key("https://s3.example.com:9000"),
            endpoint_key("https://s3.example.com")
        );
    }

    #[test]
    fn a_path_folds_dots_and_windows_spelling() {
        assert_eq!(path_key("/srv/./repo"), "/srv/repo");
        assert_ne!(path_key("/srv/Repo"), path_key("/srv/repo"));
        assert_eq!(path_key("C:\\Backup\\Repo"), path_key("c:/backup/repo"));
    }
}
