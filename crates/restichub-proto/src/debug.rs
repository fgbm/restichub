//! Manual `Debug` implementations that keep secrets out of output.
//!
//! The build script skips the derived `Debug` for the messages that carry
//! secrets (FR-PR-10, AC-AG-03). These implementations print the non-secret
//! fields and "***" for each secret field. `SignedJob` prints the key
//! identifier and the byte lengths only.

use std::fmt;

use crate::agent::v1::{
    RepositoryAccess, RestCredentials, S3Credentials, SftpCredentials, SignedJob,
    repository_access::BackendCredentials,
};

const MASK: &str = "***";

impl fmt::Debug for SignedJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SignedJob")
            .field("signer_key_id", &self.signer_key_id)
            .field("job_bytes", &self.job.len())
            .field("signature_bytes", &self.signature.len())
            .finish()
    }
}

impl fmt::Debug for RepositoryAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepositoryAccess")
            .field("repository_id", &self.repository_id)
            .field("location", &self.location)
            .field("password", &MASK)
            .field("backend_credentials", &self.backend_credentials)
            .finish()
    }
}

impl fmt::Debug for BackendCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::S3(credentials) => fmt::Debug::fmt(credentials, formatter),
            Self::Rest(credentials) => fmt::Debug::fmt(credentials, formatter),
            Self::Sftp(credentials) => fmt::Debug::fmt(credentials, formatter),
            Self::Local(credentials) => fmt::Debug::fmt(credentials, formatter),
        }
    }
}

impl fmt::Debug for S3Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S3Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &MASK)
            .field("session_token", &MASK)
            .field("region", &self.region)
            .finish()
    }
}

impl fmt::Debug for RestCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestCredentials")
            .field("username", &self.username)
            .field("password", &MASK)
            .finish()
    }
}

impl fmt::Debug for SftpCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SftpCredentials")
            .field("private_key", &MASK)
            .field("password", &MASK)
            .field("known_hosts_entry", &self.known_hosts_entry)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    use crate::agent::v1::{ControlMessage, Job, control_message, repository_access};

    const CANARY: &str = "CANARY-do-not-log";

    fn secret_repository() -> RepositoryAccess {
        RepositoryAccess {
            repository_id: "repo-1".to_owned(),
            location: "s3:s3.example.com/bucket".to_owned(),
            password: CANARY.to_owned(),
            backend_credentials: Some(repository_access::BackendCredentials::S3(S3Credentials {
                access_key_id: "access".to_owned(),
                secret_access_key: CANARY.to_owned(),
                session_token: CANARY.to_owned(),
                region: "eu-west-1".to_owned(),
            })),
        }
    }

    #[test]
    fn ac_ag_03_debug_output_has_no_secrets() {
        let job = Job {
            job_id: "job-1".to_owned(),
            repositories: vec![secret_repository()],
            ..Job::default()
        };
        assert!(!format!("{job:?}").contains(CANARY));

        let control = ControlMessage {
            payload: Some(control_message::Payload::SignedJob(SignedJob {
                job: job.encode_to_vec(),
                signature: vec![0; 64],
                signer_key_id: "key-1".to_owned(),
            })),
        };
        assert!(!format!("{control:?}").contains(CANARY));
    }
}
