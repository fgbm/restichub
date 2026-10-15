//! Removal of secrets from restic text before it leaves the agent (FR-AG-06, FR-AG-08, threat T9).
//!
//! restic does not print the password, but an error message can carry the repository location with
//! its userinfo, and a backend can echo a credential. The scrubber knows every secret value of one
//! step and replaces each of them, so no log line, journal entry or error holds one.

use restichub_compiler::arg::{CommandLine, MASK, is_safe_env};
use zeroize::Zeroizing;

/// Name of the variable that carries the location, with its userinfo.
const REPOSITORY_ENV: &str = "RESTIC_REPOSITORY";

/// Name of the variable that carries the user name of a REST backend.
const REST_USERNAME_ENV: &str = "RESTIC_REST_USERNAME";

/// Holds every secret value of one step and masks them in restic text.
pub(crate) struct Scrubber {
    /// Passwords, keys and tokens, the longest first, so a secret that holds another one is
    /// replaced whole. They are masked in any context, whatever their length.
    secrets: Vec<Zeroizing<String>>,
    /// User names, each with the URL forms in which it is masked. A user name is a common word
    /// (`backup`), so it is masked only in a URL context.
    names: Vec<NamePattern>,
    /// Repository URLs that carry userinfo, each with its masked form.
    locations: Vec<(Zeroizing<String>, String)>,
}

/// The URL forms of one user name and their masked replacement.
struct NamePattern {
    forms: [(Zeroizing<String>, String); 2],
}

impl NamePattern {
    fn new(name: &str) -> Self {
        Self {
            forms: [
                (Zeroizing::new(format!("://{name}:")), format!("://{MASK}:")),
                (Zeroizing::new(format!("{name}@")), format!("{MASK}@")),
            ],
        }
    }
}

impl Scrubber {
    /// Collects the secret values from the environment of the step command.
    pub(crate) fn for_command(line: &CommandLine) -> Self {
        let mut secrets: Vec<Zeroizing<String>> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        let mut locations = Vec::new();
        for (name, value) in line.env() {
            if name == REPOSITORY_ENV {
                if let Some((masked, userinfo)) = mask_location(value) {
                    locations.push((Zeroizing::new(value.clone()), masked));
                    match userinfo.split_once(':') {
                        Some((user, password)) => {
                            names.push(percent_decode(user).unwrap_or_else(|| user.to_owned()));
                            names.push(user.to_owned());
                            secrets.push(Zeroizing::new(password.to_owned()));
                            // restic prints the decoded password when the backend rejects it.
                            if let Some(decoded) = percent_decode(password) {
                                secrets.push(Zeroizing::new(decoded));
                            }
                        }
                        None => secrets.push(Zeroizing::new(userinfo.to_owned())),
                    }
                    secrets.push(Zeroizing::new(userinfo.to_owned()));
                }
            } else if name == REST_USERNAME_ENV {
                names.push(value.clone());
            } else if !is_safe_env(name) {
                secrets.push(Zeroizing::new(value.clone()));
            }
        }
        secrets.retain(|secret| !secret.is_empty());
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        names.retain(|name| !name.is_empty());
        names.sort();
        names.dedup();
        Self {
            secrets,
            names: names.iter().map(|name| NamePattern::new(name)).collect(),
            locations,
        }
    }

    /// Holds the secret values of one hook, so the hook output keeps none of them (FR-HK-03).
    pub(crate) fn for_secrets(values: &[Zeroizing<String>]) -> Self {
        let mut secrets: Vec<Zeroizing<String>> = values
            .iter()
            .filter(|secret| !secret.is_empty())
            .cloned()
            .collect();
        // The reader splits the output into lines, so a secret with line breaks never matches as a
        // whole. Each of its lines is masked too.
        for secret in values {
            if secret.contains('\n') {
                secrets.extend(
                    secret
                        .lines()
                        .map(str::trim)
                        .filter(|line| line.len() >= 6)
                        .map(|line| Zeroizing::new(line.to_owned())),
                );
            }
        }
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        Self {
            secrets,
            names: Vec::new(),
            locations: Vec::new(),
        }
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
        for pattern in &self.names {
            for (form, masked) in &pattern.forms {
                if text.contains(form.as_str()) {
                    text = text.replace(form.as_str(), masked);
                }
            }
        }
        text
    }

    /// A line that the reader cut can end inside a secret, so the end of a cut line loses any
    /// part of a secret too (FR-AG-06).
    pub(crate) fn scrub_line(&self, text: &str, cut: bool) -> String {
        let mut text = self.scrub(text);
        if cut {
            self.mask_cut_end(&mut text);
        }
        text
    }

    fn mask_cut_end(&self, text: &mut String) {
        for secret in &self.secrets {
            let secret = secret.as_str();
            let starts: Vec<usize> = secret.char_indices().map(|(index, _)| index).collect();
            for &end in starts.iter().skip(1).rev() {
                if text.ends_with(&secret[..end]) {
                    text.truncate(text.len() - end);
                    text.push_str(MASK);
                    return;
                }
            }
        }
    }
}

/// Decodes `%XX` sequences. `None` when the text has none or the result is not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    if !text.contains('%') {
        return None;
    }
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .filter(|_| bytes[index] == b'%')
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        if let Some(byte) = hex {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
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
    fn fr_ag_06_a_user_name_is_masked_only_in_a_url_context() {
        let scrubber = scrubber("rest:https://backup:CANARY-rest@host.example/repo", None);
        assert_eq!(
            scrubber.scrub("error during backup of /srv: backup finished"),
            "error during backup of /srv: backup finished"
        );
        assert_eq!(
            scrubber.scrub("see https://backup:other@elsewhere/x and backup@mail.example"),
            "see https://***:other@elsewhere/x and ***@mail.example"
        );
        let rest = BackendCredentials::Rest(RestCredentials {
            username: "bob".to_owned(),
            password: "pw".to_owned(),
        });
        let scrubber = self::scrubber("rest:https://host/", Some(rest));
        // A short password is masked everywhere, a short user name only in a URL.
        assert_eq!(scrubber.scrub("bob has pw"), "bob has ***");
        assert_eq!(scrubber.scrub("at https://bob@host"), "at https://***@host");
    }

    #[test]
    fn fr_ag_06_the_decoded_password_of_the_location_is_masked() {
        let scrubber = scrubber("rest:https://user:p%40ss%3Aword%21@host.example/repo", None);
        let scrubbed = scrubber.scrub("backend said: bad credentials p@ss:word! for user");
        assert!(!scrubbed.contains("p@ss:word!"), "{scrubbed}");
    }

    #[test]
    fn fr_ag_06_a_secret_cut_by_the_line_limit_is_masked() {
        let scrubber = scrubber("/srv/repo", None);
        // The reader cut this line inside the password.
        let cut = scrubber.scrub_line("Fatal: bad password CANARY-pas", true);
        assert_eq!(cut, "Fatal: bad password ***");
        // A line that the reader did not cut keeps its ending.
        assert_eq!(
            scrubber.scrub_line("value CANARY-pas", false),
            "value CANARY-pas"
        );
    }

    #[test]
    fn a_text_without_a_secret_is_unchanged() {
        let scrubber = scrubber("/srv/repo", None);
        assert_eq!(scrubber.scrub("scanning /srv/repo"), "scanning /srv/repo");
    }
}
