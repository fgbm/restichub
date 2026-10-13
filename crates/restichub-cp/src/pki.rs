//! The private certificate authority of the installation (ADR-0026).
//!
//! The installation has a two-level certificate authority. One root CA (10 years) signs one issuing
//! CA (2 years); the issuing CA signs the agent certificates and the control plane server
//! certificate. The agents trust the root CA.
//!
//! The control plane wraps each CA private key with the key encryption key (KEK) and stores both
//! rows in the platform tenant ([`restichub_db::ca`]). The certificates are stored in clear. The
//! control plane stores no agent private key.
//!
//! The service loads the wrapped keys only. It unwraps a key on demand when it signs, so no
//! plaintext key lives for the lifetime of the process. [`CaService::ensure_initialized`] unwraps
//! each key once at start to prove that the KEK and the stored bytes are correct, then drops the
//! plaintext.
//!
//! The issuing CA rotates with an overlap: [`CaService::rotate_issuing_ca`] signs a new issuing CA
//! with the root and keeps the older one until its certificate expires (ADR-0026).

use std::sync::Arc;

use rcgen::string::Ia5String;
use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256, SanType,
};
use restichub_db::ca::{self, CaKeyRow, NewCaKey};
use restichub_db::{Database, PLATFORM_TENANT_ID, TenantTx};
use restichub_secrets::{
    CaRole, CryptoError, KekError, KekId, KekProvider, WrappedCaKey, unwrap_ca_key, wrap_ca_key,
};
use rustls_pki_types::{CertificateDer, CertificateSigningRequestDer, PrivatePkcs8KeyDer};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use zeroize::Zeroizing;

/// How long the installation root CA stays valid (ADR-0026).
pub const ROOT_CA_LIFETIME: Duration = Duration::days(3650);
/// How long an issuing CA stays valid (ADR-0026).
pub const ISSUING_CA_LIFETIME: Duration = Duration::days(730);
/// The default lifetime of an agent certificate (FR-ID-03).
pub const AGENT_CERT_LIFETIME: Duration = Duration::days(30);
/// The lifetime of the control plane server certificate.
pub const SERVER_CERT_LIFETIME: Duration = Duration::days(365);

/// The advisory lock scope that serializes the creation of the installation CA.
const CA_LOCK: &str = "restichub.ca_init:";

/// Builds the identity URI of an agent (ADR-0026, section Identity).
pub fn agent_san_uri(tenant: Uuid, agent: Uuid) -> String {
    format!("restichub://tenant/{tenant}/agent/{agent}")
}

#[derive(Debug, thiserror::Error)]
pub enum CaError {
    #[error(transparent)]
    Database(#[from] restichub_db::DbError),
    #[error(transparent)]
    Kek(#[from] KekError),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("the certificate generation failed")]
    Certificate(#[source] rcgen::Error),
    #[error("the certificate signing request is not valid")]
    BadCsr(#[source] rcgen::Error),
    #[error("the certificate signing request asks for a certificate authority certificate")]
    CsrCaFlag,
    #[error(
        "the certificate signing request asks for a subject alternative name that the control plane does not set"
    )]
    CsrExtraSan,
    #[error("the installation has no certificate authority yet")]
    Missing,
    #[error("the installation has no valid issuing certificate authority")]
    NoIssuingCa,
    #[error("the stored certificate authority row is not valid: {cause}")]
    BadStoredCa { cause: String },
    #[error("the subject alternative name `{value}` is not valid")]
    BadSan { value: String },
}

/// An issued certificate and its chain. The type holds no private key, so a `Debug` is safe.
#[derive(Debug, Clone)]
pub struct IssuedCertificate {
    pub cert_der: Vec<u8>,
    /// The chain holds the issuing CA then the root CA (ADR-0026).
    pub chain_der: Vec<Vec<u8>>,
}

/// The control plane server certificate with its private key.
pub struct IssuedServerCertificate {
    pub cert_der: Vec<u8>,
    /// The chain holds the issuing CA then the root CA (ADR-0026).
    pub chain_der: Vec<Vec<u8>>,
    /// The server's PKCS#8 material, zeroized on drop.
    pub key_der: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for IssuedServerCertificate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedServerCertificate")
            .field("cert_der", &self.cert_der)
            .field("chain_der", &self.chain_der)
            .field("key_der", &"<redacted>")
            .finish()
    }
}

/// One loaded certificate authority. The type holds the wrapped private key and the certificate,
/// never a plaintext key.
#[derive(Debug)]
pub struct LoadedCa {
    pub id: Uuid,
    pub role: CaRole,
    pub cert_der: Vec<u8>,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
    wrapped: WrappedCaKey,
}

impl LoadedCa {
    fn from_row(row: CaKeyRow) -> Result<Self, CaError> {
        let role = CaRole::parse(&row.role).ok_or_else(|| CaError::BadStoredCa {
            cause: format!("the role `{}` is not known", row.role),
        })?;
        let kek_id = KekId::parse(&row.kek_id)?;
        let nonce = row
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| CaError::BadStoredCa {
                cause: "the nonce is not 96 bits".to_owned(),
            })?;
        Ok(Self {
            id: row.id,
            role,
            cert_der: row.cert_der,
            not_before: row.not_before,
            not_after: row.not_after,
            wrapped: WrappedCaKey {
                kek_id,
                nonce,
                ciphertext: row.wrapped_key,
            },
        })
    }

    /// Opens the stored key on demand; the plaintext lives in a zeroizing buffer only.
    fn unwrap_key(&self, provider: &dyn KekProvider) -> Result<Zeroizing<Vec<u8>>, CaError> {
        Ok(unwrap_ca_key(
            provider,
            PLATFORM_TENANT_ID.as_uuid(),
            self.role,
            &self.wrapped,
        )?)
    }

    fn issuer(&self, provider: &dyn KekProvider) -> Result<Issuer<'static, KeyPair>, CaError> {
        let key = key_pair_from_der(&self.unwrap_key(provider)?)?;
        let cert = CertificateDer::from(self.cert_der.clone());
        Issuer::from_ca_cert_der(&cert, key).map_err(CaError::Certificate)
    }
}

/// The loaded certificate authorities of the installation.
#[derive(Debug)]
pub struct CaStore {
    root: LoadedCa,
    /// Sorted by start time. A rotation leaves more than one row.
    issuing: Vec<LoadedCa>,
}

impl CaStore {
    fn from_rows(rows: Vec<CaKeyRow>) -> Result<Self, CaError> {
        let mut root = None;
        let mut issuing = Vec::new();
        for row in rows {
            let loaded = LoadedCa::from_row(row)?;
            match loaded.role {
                CaRole::Root => {
                    if root.is_some() {
                        return Err(CaError::BadStoredCa {
                            cause: "more than one root certificate authority".to_owned(),
                        });
                    }
                    root = Some(loaded);
                }
                CaRole::Issuing => issuing.push(loaded),
            }
        }
        let root = root.ok_or(CaError::Missing)?;
        if issuing.is_empty() {
            return Err(CaError::Missing);
        }
        issuing.sort_by_key(|ca| ca.not_before);
        Ok(Self { root, issuing })
    }

    pub fn root(&self) -> &LoadedCa {
        &self.root
    }

    pub fn issuing(&self) -> &[LoadedCa] {
        &self.issuing
    }

    /// The issuing CA with the latest start time that has not expired.
    ///
    /// A rotation adds a newer issuing CA; the older one stays valid until its certificate expires
    /// (ADR-0026).
    pub fn current_issuing(&self) -> Option<&LoadedCa> {
        let now = OffsetDateTime::now_utc();
        self.issuing.iter().rev().find(|ca| ca.not_after > now)
    }

    /// Unwraps each private key once and drops the plaintext.
    ///
    /// The function proves that the key encryption key and the stored bytes are correct, so an
    /// incorrect KEK or an unreadable key refuses the start (ADR-0026).
    pub fn verify(&self, provider: &dyn KekProvider) -> Result<(), CaError> {
        for ca in std::iter::once(&self.root).chain(self.issuing.iter()) {
            ca.unwrap_key(provider)?;
        }
        Ok(())
    }
}

/// The certificate authority of the installation.
#[derive(Debug, Clone)]
pub struct CaService {
    provider: Arc<dyn KekProvider>,
}

impl CaService {
    pub fn new(provider: Arc<dyn KekProvider>) -> Self {
        Self { provider }
    }

    /// Reads the certificate authorities of the platform tenant.
    ///
    /// The function reads the wrapped keys only. `None` when the installation has no CA yet.
    pub async fn load(&self, db: &Database) -> Result<Option<CaStore>, CaError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        let rows = ca::list(&mut tx).await?;
        tx.commit().await?;
        if rows.is_empty() {
            return Ok(None);
        }
        Ok(Some(CaStore::from_rows(rows)?))
    }

    /// Creates the root CA and the first issuing CA when the platform tenant holds none.
    ///
    /// An advisory lock serializes two starts, so exactly one start creates the CA. The function
    /// then reads the rows and unwraps each key once to prove that the KEK and the stored bytes are
    /// correct; the plaintext is dropped at once.
    pub async fn ensure_initialized(&self, db: &Database) -> Result<CaStore, CaError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        // The lock serializes two starts. The read below happens after the lock, so the second
        // start sees the rows that the first start created.
        tx.lock(CA_LOCK).await?;
        let existing = ca::list(&mut tx).await?;
        if existing.is_empty() {
            let (root, issuing) = self.generate_installation()?;
            self.insert(&mut tx, &root).await?;
            self.insert(&mut tx, &issuing).await?;
        }
        tx.commit().await?;
        let store = self.load(db).await?.ok_or(CaError::Missing)?;
        store.verify(self.provider.as_ref())?;
        Ok(store)
    }

    /// Signs a certificate signing request into an agent certificate with the current issuing CA.
    ///
    /// The certificate carries the SAN URI `restichub://tenant/<tenant>/agent/<agent>` and the agent
    /// UUID as the common name. The lifetime is [`AGENT_CERT_LIFETIME`] and the key usage is
    /// `clientAuth` (FR-ID-02, FR-ID-03). A request that asks for a CA certificate or for another
    /// subject alternative name is refused.
    pub fn issue_agent_certificate(
        &self,
        store: &CaStore,
        tenant: Uuid,
        agent: Uuid,
        csr_der: &[u8],
    ) -> Result<IssuedCertificate, CaError> {
        let expected_uri = agent_san_uri(tenant, agent);
        let csr =
            CertificateSigningRequestParams::from_der(&CertificateSigningRequestDer::from(csr_der))
                .map_err(CaError::BadCsr)?;
        validate_csr(&csr, &expected_uri)?;

        let issuing = store.current_issuing().ok_or(CaError::NoIssuingCa)?;
        let issuer = issuing.issuer(self.provider.as_ref())?;
        let san = Ia5String::try_from(expected_uri.clone()).map_err(|_| CaError::BadSan {
            value: expected_uri.clone(),
        })?;

        let now = OffsetDateTime::now_utc();
        let mut params = CertificateParams::default();
        params.distinguished_name = distinguished_name(&agent.to_string());
        params.subject_alt_names = vec![SanType::URI(san)];
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.use_authority_key_identifier_extension = true;
        params.not_before = now;
        params.not_after = now + AGENT_CERT_LIFETIME;

        let cert = params
            .signed_by(&csr.public_key, &issuer)
            .map_err(CaError::Certificate)?;
        Ok(IssuedCertificate {
            cert_der: cert.der().to_vec(),
            chain_der: vec![issuing.cert_der.clone(), store.root.cert_der.clone()],
        })
    }

    /// Issues the control plane server certificate for the agent listener (ADR-0026).
    ///
    /// The subject alternative names come from the configuration; a name is a DNS name or an IP
    /// address. The key usage is `serverAuth`.
    pub fn issue_server_certificate(
        &self,
        store: &CaStore,
        sans: &[String],
    ) -> Result<IssuedServerCertificate, CaError> {
        let issuing = store.current_issuing().ok_or(CaError::NoIssuingCa)?;
        let issuer = issuing.issuer(self.provider.as_ref())?;

        let now = OffsetDateTime::now_utc();
        let mut params = CertificateParams::new(sans.to_vec()).map_err(CaError::Certificate)?;
        params.distinguished_name = distinguished_name("ResticHub Control Plane");
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        params.not_before = now;
        params.not_after = now + SERVER_CERT_LIFETIME;

        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(CaError::Certificate)?;
        let cert = params
            .signed_by(&key, &issuer)
            .map_err(CaError::Certificate)?;
        Ok(IssuedServerCertificate {
            cert_der: cert.der().to_vec(),
            chain_der: vec![issuing.cert_der.clone(), store.root.cert_der.clone()],
            key_der: Zeroizing::new(key.serialize_der()),
        })
    }

    /// Signs a new issuing CA with the root and stores it.
    ///
    /// The older issuing CA stays in the store and stays valid until its certificate expires; the
    /// control plane accepts both during the overlap (ADR-0026). The trigger of a rotation is out
    /// of scope.
    pub async fn rotate_issuing_ca(
        &self,
        db: &Database,
        store: &CaStore,
    ) -> Result<CaStore, CaError> {
        let now = OffsetDateTime::now_utc();
        let root_issuer = store.root().issuer(self.provider.as_ref())?;
        let pending = new_ca(
            CaRole::Issuing,
            ISSUING_CA_COMMON_NAME,
            ISSUING_CA_LIFETIME,
            Some(&root_issuer),
            now,
        )?;

        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        tx.lock(CA_LOCK).await?;
        self.insert(&mut tx, &pending).await?;
        tx.commit().await?;
        self.load(db).await?.ok_or(CaError::Missing)
    }

    fn generate_installation(&self) -> Result<(PendingCa, PendingCa), CaError> {
        let now = OffsetDateTime::now_utc();
        let root = new_ca(
            CaRole::Root,
            ROOT_CA_COMMON_NAME,
            ROOT_CA_LIFETIME,
            None,
            now,
        )?;
        let root_issuer = root.issuer()?;
        let issuing = new_ca(
            CaRole::Issuing,
            ISSUING_CA_COMMON_NAME,
            ISSUING_CA_LIFETIME,
            Some(&root_issuer),
            now,
        )?;
        Ok((root, issuing))
    }

    async fn insert(&self, tx: &mut TenantTx<'_>, pending: &PendingCa) -> Result<(), CaError> {
        let wrapped = wrap_ca_key(
            self.provider.current(),
            PLATFORM_TENANT_ID.as_uuid(),
            pending.role,
            &pending.key_der,
        )?;
        let new = NewCaKey {
            id: pending.id,
            role: pending.role.as_str(),
            kek_id: wrapped.kek_id.as_str(),
            nonce: &wrapped.nonce,
            wrapped_key: &wrapped.ciphertext,
            cert_der: &pending.cert_der,
            not_before: pending.not_before,
            not_after: pending.not_after,
        };
        ca::insert(tx, &new).await?;
        Ok(())
    }
}

const ROOT_CA_COMMON_NAME: &str = "ResticHub Installation Root CA";
const ISSUING_CA_COMMON_NAME: &str = "ResticHub Installation Issuing CA";

/// A certificate authority that is generated but not yet stored. The type holds the plaintext key
/// in a zeroizing buffer only, so it derives no `Debug`.
struct PendingCa {
    id: Uuid,
    role: CaRole,
    cert_der: Vec<u8>,
    key_der: Zeroizing<Vec<u8>>,
    not_before: OffsetDateTime,
    not_after: OffsetDateTime,
}

impl PendingCa {
    fn key_pair(&self) -> Result<KeyPair, CaError> {
        key_pair_from_der(&self.key_der)
    }

    fn issuer(&self) -> Result<Issuer<'static, KeyPair>, CaError> {
        let cert = CertificateDer::from(self.cert_der.clone());
        Issuer::from_ca_cert_der(&cert, self.key_pair()?).map_err(CaError::Certificate)
    }
}

fn new_ca(
    role: CaRole,
    common_name: &str,
    lifetime: Duration,
    issuer: Option<&Issuer<'_, KeyPair>>,
    now: OffsetDateTime,
) -> Result<PendingCa, CaError> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(CaError::Certificate)?;
    let mut params = CertificateParams::default();
    params.distinguished_name = distinguished_name(common_name);
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params.use_authority_key_identifier_extension = true;
    params.not_before = now;
    params.not_after = now + lifetime;

    let cert = match issuer {
        Some(issuer) => params
            .signed_by(&key, issuer)
            .map_err(CaError::Certificate)?,
        None => params.self_signed(&key).map_err(CaError::Certificate)?,
    };
    Ok(PendingCa {
        id: Uuid::now_v7(),
        role,
        cert_der: cert.der().to_vec(),
        key_der: Zeroizing::new(key.serialize_der()),
        not_before: now,
        not_after: now + lifetime,
    })
}

fn key_pair_from_der(der: &[u8]) -> Result<KeyPair, CaError> {
    KeyPair::from_pkcs8_der_and_sign_algo(&PrivatePkcs8KeyDer::from(der), &PKCS_ECDSA_P256_SHA256)
        .map_err(CaError::Certificate)
}

fn distinguished_name(common_name: &str) -> DistinguishedName {
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, common_name);
    name.push(DnType::OrganizationName, "ResticHub");
    name
}

/// Refuses a request that asks for a CA certificate or for a subject alternative name that the
/// control plane does not set (ADR-0026, section Identity).
fn validate_csr(csr: &CertificateSigningRequestParams, expected_uri: &str) -> Result<(), CaError> {
    if !matches!(csr.params.is_ca, IsCa::NoCa) {
        return Err(CaError::CsrCaFlag);
    }
    for san in &csr.params.subject_alt_names {
        let allowed = matches!(san, SanType::URI(uri) if uri.as_str() == expected_uri);
        if !allowed {
            return Err(CaError::CsrExtraSan);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{AGENT_CERT_LIFETIME, ISSUING_CA_LIFETIME, ROOT_CA_LIFETIME, agent_san_uri};

    #[test]
    fn the_agent_san_uri_binds_the_tenant_and_the_agent() {
        let tenant = Uuid::from_u128(0x0189_1234_5678_9abc_def0_1122_3344_5566);
        let agent = Uuid::from_u128(0x0189_aaaa_bbbb_cccc_dddd_eeee_ffff_0000);
        let uri = agent_san_uri(tenant, agent);
        assert_eq!(uri, format!("restichub://tenant/{tenant}/agent/{agent}"));
    }

    #[test]
    fn the_ca_lifetimes_match_the_decision() {
        assert_eq!(ROOT_CA_LIFETIME.whole_days(), 3650);
        assert_eq!(ISSUING_CA_LIFETIME.whole_days(), 730);
        assert_eq!(AGENT_CERT_LIFETIME.whole_days(), 30);
    }
}
