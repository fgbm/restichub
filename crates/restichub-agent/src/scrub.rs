//! Removal of secrets from restic text before it leaves the agent (FR-AG-06, FR-AG-08, threat T9).
//!
//! restic does not print the password, but an error message can carry the repository location with
//! its userinfo, and a backend can echo a credential. The scrubber knows every secret value of one
//! step and replaces each of them, so no log line, journal entry or error holds one.

use restichub_compiler::arg::{CommandLine, MASK, is_safe_env};
use zeroize::Zeroizing;

/// Name of the variable that carries the location, with its userinfo.
const REPOSITORY_ENV: &str = "RESTIC_REPOSITORY";

/// Holds every secret value of one step and masks them in restic text.
pub(crate) struct Scrubber {
    /// Secret values, the longest first, so a secret that holds another one is replaced whole.
    secrets: Vec<Zeroizing<String>>,
    /// Repository URLs that carry userinfo, each with its masked form.
    locations: Vec<(Zeroizing<String>, String)>,
}

impl Scrubber {
    /// Collects the secret values from the environment of the step command.
    pub(crate) fn for_command(line: &CommandLine) -> Self {
        let mut secrets: Vec<Zeroizing<String>> = Vec::new();
        let mut locations = Vec::new();
        for (name, value) in line.env() {
            if name == REPOSITORY_ENV {
                if let Some((masked, userinfo)) = mask_location(value) {
                    locations.push((Zeroizing::new(value.clone()), masked));
                    for part in userinfo.split(':') {
                        secrets.push(Zeroizing::new(part.to_owned()));
                    }
                    secrets.push(Zeroizing::new(userinfo.to_owned()));
                }
            } else if !is_safe_env(name) {
                secrets.push(Zeroizing::new(value.clone()));
            }
        }
        secrets.retain(|secret| !secret.is_empty());
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        Self { secrets, locations }
    }

    /// Returns the text with every known secret replaced by the mask.
    pub(crate) fn scrub(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for (raw, masked) in &self.locations {
            if text.contains(raw.as_str()) {
                text = text.replace(raw.as_str(), masked);
            }
        }
        for secret in &self.secrets {
            if text.contains(secret.as_str()) {
                text = text.replace(secret.as_str(), MASK);
            }
        }
        text
    }
}

/// Splits a location with userinfo into the masked location and the userinfo. A location without
/// userinfo gives `None`.
fn mask_location(location: &str) -> Option<(String, &str)> {
    let scheme_end = location.find("://")?;
    let after = &location[scheme_end + 3..];
    let at = after.find('@')?;
    if at >= after.find('/').unwrap_or(usize::MAX) {
        return None;
    }
    let masked = format!("{}{MASK}{}", &location[..scheme_end + 3], &after[at..]);
    Some((masked, &after[..at]))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use restichub_compiler::ValidatedJob;
    use restichub_compiler::arg::build_command;
    use restichub_proto::agent::v1::{
        Job, RepositoryAccess, RestCredentials, S3Credentials, Step, UnlockStep,
        repository_access::BackendCredentials, step,
    };

    use super::Scrubber;

    fn scrubber(location: &str, credentials: Option<BackendCredentials>) -> Scrubber {
        let step = Step {
            step_id: "unlock".to_owned(),
            kind: Some(step::Kind::Unlock(UnlockStep {
                repository_id: "repo-1".to_owned(),
            })),
            ..Step::default()
        };
        let job = Job {
            job_id: "job-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            repositories: vec![RepositoryAccess {
                repository_id: "repo-1".to_owned(),
                location: location.to_owned(),
                password: "CANARY-password".to_owned(),
                backend_credentials: credentials,
            }],
            steps: vec![step.clone()],
            ..Job::default()
        };
        let validated = ValidatedJob::new(job).expect("the job must validate");
        Scrubber::for_command(&build_command(&validated, &step).expect("the command must build"))
    }

    #[test]
    fn fr_ag_06_the_password_never_survives() {
        let scrubber = scrubber("/srv/repo", None);
        let text = "Fatal: wrong password CANARY-password, again CANARY-password";
        assert_eq!(scrubber.scrub(text), "Fatal: wrong password ***, again ***");
    }

    #[test]
    fn fr_ag_06_the_userinfo_of_the_location_is_masked() {
        let scrubber = scrubber("rest:https://backup:CANARY-rest@host.example/repo", None);
        let text = "Fatal: unable to open repository at rest:https://backup:CANARY-rest@host.example/repo: 401";
        let scrubbed = scrubber.scrub(text);
        assert!(!scrubbed.contains("CANARY-rest"), "{scrubbed}");
        assert!(scrubbed.contains("rest:https://***@host.example/repo"));
        // A part of the userinfo that restic prints alone is masked too.
        assert!(!scrubber.scrub("user CANARY-rest").contains("CANARY-rest"));
    }

    #[test]
    fn fr_ag_06_the_backend_credentials_are_masked() {
        let s3 = BackendCredentials::S3(S3Credentials {
            access_key_id: "AKIACANARY".to_owned(),
            secret_access_key: "CANARY-secret-key".to_owned(),
            session_token: "CANARY-token".to_owned(),
            region: "eu-west-1".to_owned(),
        });
        let scrubber = scrubber("s3:s3.example.com/bucket", Some(s3));
        let scrubbed = scrubber.scrub("key AKIACANARY secret CANARY-secret-key t CANARY-token");
        assert_eq!(scrubbed, "key *** secret *** t ***");
        // Region names are not secret.
        assert_eq!(scrubber.scrub("region eu-west-1"), "region eu-west-1");
        let rest = BackendCredentials::Rest(RestCredentials {
            username: "alice".to_owned(),
            password: "CANARY-rest-pass".to_owned(),
        });
        let scrubber = self::scrubber("rest:https://host/", Some(rest));
        assert_eq!(scrubber.scrub("pw CANARY-rest-pass"), "pw ***");
    }

    #[test]
    fn a_text_without_a_secret_is_unchanged() {
        let scrubber = scrubber("/srv/repo", None);
        assert_eq!(scrubber.scrub("scanning /srv/repo"), "scanning /srv/repo");
    }
}
