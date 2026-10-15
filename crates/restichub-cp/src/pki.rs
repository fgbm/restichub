//! The private certificate authority of the installation (ADR-0026).
//!
//! The installation has a two-level certificate authority. One root CA (10 years, path length 1)
//! signs one issuing CA (2 years, path length 0); the issuing CA signs the agent certificates and
//! the control plane server certificate. The agents trust the root CA.
//!
//! The control plane wraps each CA private key with the key encryption key (KEK) and stores both
//! rows in the platform tenant ([`restichub_db::ca`]). The certificates are stored in clear. The
//! control plane stores no agent private key.
//!
//! Every certificate carries a random 16-byte serial number. The binding of a stored row is checked
//! at load: the public key of the unwrapped private key must equal the key of the certificate, the
//! basic constraints must match the role, and each issuing CA must chain to the root. The validity
//! comes from the certificate, never from the stored columns. A swapped or replaced blob refuses
//! the start (T19).
//!
//! The service keeps one [`Issuer`] for the current issuing CA in memory, so each issue reuses it
//! instead of rebuilding the key. The root key is unwrapped on demand, only for a rotation.
//!
//! The issuing CA rotates with an overlap: [`CaService::rotate_issuing_ca`] signs a new issuing CA
//! with the root and keeps the older one until its certificate expires. [`CaService::ensure_initialized`]
//! rotates at start when the current issuing CA has less than [`ISSUING_CA_RENEWAL_WINDOW`] left,
//! and a leaf never outlives its issuing CA.
//!
//! An issued certificate carries the chain of the issuing CA and then the root CA.
//!
//! # Residual risk
//!
//! `rcgen` and `ring` keep the private key bytes of a [`KeyPair`] in the heap in a buffer that they
//! do not zeroize (`rcgen` zeroizes the serialized PKCS#8 copy on drop, not every working copy).
//! The control plane reduces the exposure: it holds the plaintext only for the current issuing CA,
//! it drops every other copy at load, and [`crate::harden::disable_core_dumps`] refuses a core
//! dump on Linux. A service unit sets `LimitCORE=0` as the second barrier. An attacker who can read
//! the process memory already holds the KEK path. This is the accepted residual risk next to T18.

use std::sync::Arc;

use rcgen::string::Ia5String;
use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256, PublicKeyData, SanType, SerialNumber,
};
use restichub_db::ca::{self, CaKeyRow, NewCaKey};
use restichub_db::{CaRole, Database, PLATFORM_TENANT_ID, TenantTx};
use restichub_secrets::{
    CryptoError, KekError, KekId, KekProvider, WrappedCaKey, unwrap_ca_key, wrap_ca_key,
};
use rustls_pki_types::{CertificateDer, CertificateSigningRequestDer, PrivatePkcs8KeyDer};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use x509_parser::certificate::X509Certificate;
use x509_parser::error::X509Error;
use x509_parser::nom::Err as NomErr;
use zeroize::Zeroizing;

/// How long the installation root CA stays valid (ADR-0026).
pub const ROOT_CA_LIFETIME: Duration = Duration::days(3650);
/// How long an issuing CA stays valid (ADR-0026).
pub const ISSUING_CA_LIFETIME: Duration = Duration::days(730);
/// The default lifetime of an agent certificate (FR-ID-03).
pub const AGENT_CERT_LIFETIME: Duration = Duration::days(30);
/// The lifetime of the control plane server certificate.
pub const SERVER_CERT_LIFETIME: Duration = Duration::days(365);
/// The remaining life below which a start rotates the issuing CA (ADR-0026).
pub const ISSUING_CA_RENEWAL_WINDOW: Duration = Duration::days(60);

/// The path length of the root CA: it signs one issuing CA only (ADR-0026).
const ROOT_PATH_LEN: u8 = 1;
/// The path length of an issuing CA: it signs end-entity certificates only (ADR-0026).
const ISSUING_PATH_LEN: u8 = 0;

/// The advisory lock scope that serializes the creation and the rotation of the installation CA.
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
    #[error("the operating system random number generator failed")]
    Random(#[source] getrandom::Error),
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
    #[error(
        "the certificate signing request uses a key algorithm that the control plane does not accept"
    )]
    CsrKeyAlgorithm,
    #[error("the installation has no certificate authority yet")]
    Missing,
    #[error("the installation holds more than one root certificate authority")]
    MultipleRoot,
    #[error("the installation has no valid issuing certificate authority")]
    NoIssuingCa,
    #[error(
        "the installation root certificate authority expired at {not_after}; rotate the root CA by the documented procedure"
    )]
    RootExpired { not_after: OffsetDateTime },
    #[error("the stored certificate authority {id} does not hold a valid certificate")]
    BadStoredCa {
        id: Uuid,
        #[source]
        source: NomErr<X509Error>,
    },
    #[error("the stored certificate authority {id} holds an invalid extension")]
    BadStoredCaExtension {
        id: Uuid,
        #[source]
        source: X509Error,
    },
    #[error("the stored key of the certificate authority {id} does not match its certificate")]
    KeyMismatch { id: Uuid },
    #[error("the stored certificate authority {id} does not match its role")]
    BadCaRole { id: Uuid },
    #[error("the stored nonce of the certificate authority {id} is not 96 bits")]
    BadNonce {
        id: Uuid,
        #[source]
        source: std::array::TryFromSliceError,
    },
    #[error("the root certificate authority {id} is not self-signed")]
    UntrustedRoot {
        id: Uuid,
        #[source]
        source: X509Error,
    },
    #[error("the issuing certificate authority {id} does not chain to the root")]
    UntrustedIssuingCa {
        id: Uuid,
        #[source]
        source: X509Error,
    },
    #[error("the issuing certificate authority {id} is not valid inside the validity of the root")]
    IssuingOutsideRoot { id: Uuid },
    #[error("the subject alternative name `{value}` is not valid")]
    BadSan {
        value: String,
        #[source]
        source: rcgen::Error,
    },
}

/// An issued certificate and its chain. The type holds no private key, so a `Debug` is safe.
#[derive(Debug, Clone)]
pub struct IssuedCertificate {
    pub cert_der: Vec<u8>,
    pub chain_der: Vec<Vec<u8>>,
    /// Random; the registry row stores the same bytes.
    pub serial: Vec<u8>,
    /// Taken from the issued certificate.
    pub not_after: OffsetDateTime,
}

/// The control plane server certificate with its private key.
pub struct IssuedServerCertificate {
    pub cert_der: Vec<u8>,
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

/// One loaded certificate authority. The type holds the wrapped private key and the certificate.
#[derive(Debug)]
pub struct LoadedCa {
    pub id: Uuid,
    pub role: CaRole,
    pub cert_der: Vec<u8>,
    /// Taken from the certificate, not from the stored columns.
    pub not_before: OffsetDateTime,
    /// Taken from the certificate, not from the stored columns.
    pub not_after: OffsetDateTime,
    wrapped: WrappedCaKey,
}

impl LoadedCa {
    /// Parses the certificate, unwraps the key and checks that the two agree.
    ///
    /// The returned [`KeyPair`] is the unwrapped key; the caller keeps it only for the current
    /// issuing CA.
    fn from_row(row: CaKeyRow, provider: &dyn KekProvider) -> Result<(Self, KeyPair), CaError> {
        let cert = parse_certificate(&row.cert_der, row.id)?;
        let not_before = cert.validity().not_before.to_datetime();
        let not_after = cert.validity().not_after.to_datetime();

        let wrapped = WrappedCaKey {
            kek_id: KekId::parse(&row.kek_id)?,
            nonce: row
                .nonce
                .as_slice()
                .try_into()
                .map_err(|source| CaError::BadNonce { id: row.id, source })?,
            ciphertext: row.wrapped_key,
        };
        let key = key_pair_from_der(&unwrap_ca_key(
            provider,
            PLATFORM_TENANT_ID.as_uuid(),
            row.role.as_str(),
            &wrapped,
        )?)?;
        if key.der_bytes() != cert.public_key().subject_public_key.as_ref() {
            return Err(CaError::KeyMismatch { id: row.id });
        }

        let basic = cert
            .basic_constraints()
            .map_err(|source| CaError::BadStoredCaExtension { id: row.id, source })?;
        let (is_ca, path_len) = match basic {
            Some(extension) => (extension.value.ca, extension.value.path_len_constraint),
            None => (false, None),
        };
        let role_ok = match row.role {
            CaRole::Root => is_ca && path_len.unwrap_or(0) >= u32::from(ROOT_PATH_LEN),
            CaRole::Issuing => is_ca && path_len == Some(u32::from(ISSUING_PATH_LEN)),
        };
        if !role_ok {
            return Err(CaError::BadCaRole { id: row.id });
        }

        Ok((
            Self {
                id: row.id,
                role: row.role,
                cert_der: row.cert_der,
                not_before,
                not_after,
                wrapped,
            },
            key,
        ))
    }

    /// Builds the signing authority from the stored key. The rotation uses it for the root CA.
    fn issuer(&self, provider: &dyn KekProvider) -> Result<Issuer<'static, KeyPair>, CaError> {
        let der = unwrap_ca_key(
            provider,
            PLATFORM_TENANT_ID.as_uuid(),
            self.role.as_str(),
            &self.wrapped,
        )?;
        let key = key_pair_from_der(&der)?;
        let cert = CertificateDer::from(self.cert_der.clone());
        Issuer::from_ca_cert_der(&cert, key).map_err(CaError::Certificate)
    }
}

/// The current issuing CA and the issuer built from it, kept for the process lifetime.
#[derive(Debug)]
struct CurrentCa {
    index: usize,
    issuer: Issuer<'static, KeyPair>,
}

/// The loaded certificate authorities of the installation.
#[derive(Debug)]
pub struct CaStore {
    root: LoadedCa,
    /// Sorted by start time. A rotation leaves more than one row.
    issuing: Vec<LoadedCa>,
    current: Option<CurrentCa>,
}

impl CaStore {
    fn from_rows(rows: Vec<CaKeyRow>, provider: &dyn KekProvider) -> Result<Self, CaError> {
        let mut root: Option<LoadedCa> = None;
        let mut issuing: Vec<(LoadedCa, KeyPair)> = Vec::new();
        for row in rows {
            let (ca, key) = LoadedCa::from_row(row, provider)?;
            match ca.role {
                CaRole::Root => {
                    if root.is_some() {
                        return Err(CaError::MultipleRoot);
                    }
                    // The root key is unwrapped on demand for a rotation, not kept here.
                    drop(key);
                    root = Some(ca);
                }
                CaRole::Issuing => issuing.push((ca, key)),
            }
        }
        let root = root.ok_or(CaError::Missing)?;

        let root_cert = parse_certificate(&root.cert_der, root.id)?;
        root_cert
            .verify_signature(None)
            .map_err(|source| CaError::UntrustedRoot {
                id: root.id,
                source,
            })?;
        for (ca, _) in &issuing {
            let cert = parse_certificate(&ca.cert_der, ca.id)?;
            cert.verify_signature(Some(root_cert.public_key()))
                .map_err(|source| CaError::UntrustedIssuingCa { id: ca.id, source })?;
            if ca.not_before < root.not_before || ca.not_after > root.not_after {
                return Err(CaError::IssuingOutsideRoot { id: ca.id });
            }
        }

        issuing.sort_by_key(|(ca, _)| ca.not_before);
        let now = OffsetDateTime::now_utc();
        let current_index = issuing.iter().rposition(|(ca, _)| ca.not_after > now);
        let mut current = None;
        let mut loaded = Vec::with_capacity(issuing.len());
        for (index, (ca, key)) in issuing.into_iter().enumerate() {
            if Some(index) == current_index {
                let cert = CertificateDer::from(ca.cert_der.clone());
                let issuer = Issuer::from_ca_cert_der(&cert, key).map_err(CaError::Certificate)?;
                current = Some(CurrentCa { index, issuer });
            }
            loaded.push(ca);
        }
        Ok(Self {
            root,
            issuing: loaded,
            current,
        })
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
        self.current
            .as_ref()
            .map(|current| &self.issuing[current.index])
    }

    fn current_issuer(&self) -> Option<&Issuer<'static, KeyPair>> {
        self.current.as_ref().map(|current| &current.issuer)
    }

    /// Signs a certificate signing request into an agent certificate with the current issuing CA.
    ///
    /// The certificate carries the SAN URI `restichub://tenant/<tenant>/agent/<agent>` and the agent
    /// UUID as the common name. The lifetime is [`AGENT_CERT_LIFETIME`], cut to the issuer when the
    /// issuing CA expires sooner, and the key usage is `clientAuth` (FR-ID-02, FR-ID-03). A request
    /// that asks for a CA certificate, for another subject alternative name or for a key algorithm
    /// that the control plane does not accept is refused. The issuer is in memory, so the call needs
    /// no key material from the database.
    pub fn issue_agent_certificate(
        &self,
        tenant: Uuid,
        agent: Uuid,
        csr_der: &[u8],
    ) -> Result<IssuedCertificate, CaError> {
        self.issue_agent_certificate_at(
            tenant,
            agent,
            csr_der,
            OffsetDateTime::now_utc(),
            AGENT_CERT_LIFETIME,
        )
    }

    /// Like [`Self::issue_agent_certificate`], with the issue time and the lifetime given.
    ///
    /// The rotation over the stream passes the clock of the control plane and the configured
    /// lifetime (FR-ID-03).
    pub fn issue_agent_certificate_at(
        &self,
        tenant: Uuid,
        agent: Uuid,
        csr_der: &[u8],
        now: OffsetDateTime,
        lifetime: Duration,
    ) -> Result<IssuedCertificate, CaError> {
        // A certificate holds whole seconds; the registry row stores the same expiry.
        let now = now.replace_nanosecond(0).unwrap_or(now);
        let expected_uri = agent_san_uri(tenant, agent);
        let csr =
            CertificateSigningRequestParams::from_der(&CertificateSigningRequestDer::from(csr_der))
                .map_err(CaError::BadCsr)?;
        validate_csr(&csr, &expected_uri)?;
        validate_spki_p256(csr_der)?;

        let issuing = self.current_issuing().ok_or(CaError::NoIssuingCa)?;
        let issuer = self.current_issuer().ok_or(CaError::NoIssuingCa)?;
        let san = Ia5String::try_from(expected_uri.clone()).map_err(|source| CaError::BadSan {
            value: expected_uri.clone(),
            source,
        })?;

        let mut params = CertificateParams::default();
        params.distinguished_name = distinguished_name(&agent.to_string());
        params.subject_alt_names = vec![SanType::URI(san)];
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.use_authority_key_identifier_extension = true;
        let serial = random_serial()?;
        let serial_bytes = serial.to_bytes();
        params.serial_number = Some(serial);
        params.not_before = leaf_not_before(now, lifetime, issuing.not_before);
        let not_after = leaf_not_after(now, lifetime, issuing.not_after);
        params.not_after = not_after;

        let cert = params
            .signed_by(&csr.public_key, issuer)
            .map_err(CaError::Certificate)?;
        Ok(IssuedCertificate {
            cert_der: cert.der().to_vec(),
            chain_der: vec![issuing.cert_der.clone(), self.root.cert_der.clone()],
            serial: serial_bytes,
            not_after,
        })
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
    /// Each row is checked (key, role, chain) and the validity comes from the certificates.
    /// `None` when the installation has no CA yet.
    pub async fn load(&self, db: &Database) -> Result<Option<CaStore>, CaError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        let rows = ca::list(&mut tx).await?;
        tx.commit().await?;
        if rows.is_empty() {
            return Ok(None);
        }
        Ok(Some(CaStore::from_rows(rows, self.provider.as_ref())?))
    }

    /// Creates the CA on the first start and keeps it healthy on every later start.
    ///
    /// An advisory lock serializes two starts, so exactly one start creates the CA. Under the lock
    /// the function refuses an expired root, creates an issuing CA when none exists, and rotates
    /// the issuing CA when less than [`ISSUING_CA_RENEWAL_WINDOW`] remains.
    pub async fn ensure_initialized(&self, db: &Database) -> Result<CaStore, CaError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        tx.lock(CA_LOCK).await?;
        let rows = ca::list(&mut tx).await?;
        if rows.is_empty() {
            let (root, issuing) = self.generate_installation()?;
            self.insert(&mut tx, &root).await?;
            self.insert(&mut tx, &issuing).await?;
            tx.commit().await?;
        } else {
            let store = CaStore::from_rows(rows, self.provider.as_ref())?;
            let now = OffsetDateTime::now_utc();
            if store.root().not_after <= now {
                return Err(CaError::RootExpired {
                    not_after: store.root().not_after,
                });
            }
            let renew = match store.current_issuing() {
                None => true,
                Some(ca) => ca.not_after - now < ISSUING_CA_RENEWAL_WINDOW,
            };
            if renew {
                let pending = self.new_issuing_ca(&store, now)?;
                self.insert(&mut tx, &pending).await?;
            }
            tx.commit().await?;
        }
        self.load(db).await?.ok_or(CaError::Missing)
    }

    /// Signs a certificate signing request into an agent certificate with the current issuing CA.
    pub fn issue_agent_certificate(
        &self,
        store: &CaStore,
        tenant: Uuid,
        agent: Uuid,
        csr_der: &[u8],
    ) -> Result<IssuedCertificate, CaError> {
        store.issue_agent_certificate(tenant, agent, csr_der)
    }

    /// Issues the control plane server certificate for the agent listener (ADR-0026).
    ///
    /// The subject alternative names come from the configuration; a name is a DNS name or an IP
    /// address. The key usage is `serverAuth`, and the certificate never outlives its issuer.
    pub fn issue_server_certificate(
        &self,
        store: &CaStore,
        sans: &[String],
    ) -> Result<IssuedServerCertificate, CaError> {
        let issuing = store.current_issuing().ok_or(CaError::NoIssuingCa)?;
        let issuer = store.current_issuer().ok_or(CaError::NoIssuingCa)?;

        let now = OffsetDateTime::now_utc();
        let mut params = CertificateParams::new(sans.to_vec()).map_err(CaError::Certificate)?;
        params.distinguished_name = distinguished_name("ResticHub Control Plane");
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        params.serial_number = Some(random_serial()?);
        params.not_before = now;
        params.not_after = leaf_not_after(now, SERVER_CERT_LIFETIME, issuing.not_after);

        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(CaError::Certificate)?;
        let cert = params
            .signed_by(&key, issuer)
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
    /// control plane accepts both during the overlap (ADR-0026). Under the lock the function
    /// re-reads the rows: when another start already rotated, it returns that newer CA instead of
    /// creating a second one. The trigger of a rotation is out of scope.
    pub async fn rotate_issuing_ca(
        &self,
        db: &Database,
        store: &CaStore,
    ) -> Result<CaStore, CaError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        tx.lock(CA_LOCK).await?;
        let fresh = CaStore::from_rows(ca::list(&mut tx).await?, self.provider.as_ref())?;
        let already_rotated = match (fresh.current_issuing(), store.current_issuing()) {
            (Some(fresh_ca), Some(store_ca)) => fresh_ca.id != store_ca.id,
            _ => true,
        };
        if already_rotated {
            tx.commit().await?;
            return Ok(fresh);
        }
        let pending = self.new_issuing_ca(&fresh, OffsetDateTime::now_utc())?;
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

    fn new_issuing_ca(&self, store: &CaStore, now: OffsetDateTime) -> Result<PendingCa, CaError> {
        let root_issuer = store.root().issuer(self.provider.as_ref())?;
        new_ca(
            CaRole::Issuing,
            ISSUING_CA_COMMON_NAME,
            ISSUING_CA_LIFETIME,
            Some(&root_issuer),
            now,
        )
    }

    async fn insert(&self, tx: &mut TenantTx<'_>, pending: &PendingCa) -> Result<(), CaError> {
        let wrapped = wrap_ca_key(
            self.provider.current(),
            PLATFORM_TENANT_ID.as_uuid(),
            pending.role.as_str(),
            &pending.key_der,
        )?;
        let new = NewCaKey {
            id: pending.id,
            role: pending.role,
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
    let path_len = match role {
        CaRole::Root => ROOT_PATH_LEN,
        CaRole::Issuing => ISSUING_PATH_LEN,
    };
    let mut params = CertificateParams::default();
    params.distinguished_name = distinguished_name(common_name);
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(path_len));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params.use_authority_key_identifier_extension = true;
    params.serial_number = Some(random_serial()?);
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

/// A random 16-byte serial number with the top bit cleared, so the DER integer needs no leading
/// zero and two certificates from the same key get different serials (RFC 5280, section 4.1.2.2).
fn random_serial() -> Result<SerialNumber, CaError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(CaError::Random)?;
    bytes[0] &= 0x7f;
    if bytes.iter().all(|byte| *byte == 0) {
        bytes[15] = 1;
    }
    Ok(SerialNumber::from_slice(&bytes))
}

/// How far an agent certificate starts before the issue time, so an agent with a clock that runs
/// slow still accepts it.
pub const AGENT_CERT_BACKDATE: Duration = Duration::minutes(5);

/// The issue time minus [`AGENT_CERT_BACKDATE`], but not before the issuer starts. A short lifetime
/// shortens the backdate to a quarter of it, so the half of the lifetime stays near the issue time.
fn leaf_not_before(
    now: OffsetDateTime,
    lifetime: Duration,
    issuer_not_before: OffsetDateTime,
) -> OffsetDateTime {
    (now - AGENT_CERT_BACKDATE.min(lifetime / 4)).max(issuer_not_before)
}

/// The earlier of the requested lifetime and the expiry of the issuer, so a leaf never outlives it.
fn leaf_not_after(
    now: OffsetDateTime,
    lifetime: Duration,
    issuer_not_after: OffsetDateTime,
) -> OffsetDateTime {
    (now + lifetime).min(issuer_not_after)
}

fn key_pair_from_der(der: &[u8]) -> Result<KeyPair, CaError> {
    KeyPair::from_pkcs8_der_and_sign_algo(&PrivatePkcs8KeyDer::from(der), &PKCS_ECDSA_P256_SHA256)
        .map_err(CaError::Certificate)
}

fn parse_certificate(der: &[u8], id: Uuid) -> Result<X509Certificate<'_>, CaError> {
    x509_parser::parse_x509_certificate(der)
        .map(|(_, cert)| cert)
        .map_err(|source| CaError::BadStoredCa { id, source })
}

fn distinguished_name(common_name: &str) -> DistinguishedName {
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, common_name);
    name.push(DnType::OrganizationName, "ResticHub");
    name
}

/// Refuses a request that asks for a CA certificate, for another subject alternative name, or that
/// uses a key algorithm other than ECDSA P-256 (ADR-0026, section Identity).
fn validate_csr(csr: &CertificateSigningRequestParams, expected_uri: &str) -> Result<(), CaError> {
    if csr.public_key.algorithm() != &PKCS_ECDSA_P256_SHA256 {
        return Err(CaError::CsrKeyAlgorithm);
    }
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

/// Checks the enrollment request before the token is consumed (FR-ID-02, ADR-0026).
///
/// The enrollment endpoint runs the function before the consume, so a malformed request never
/// spends a token and a request error never reveals the token state. The function verifies the
/// request signature, refuses a CA flag and a subject alternative name (the control plane sets the
/// identity URI itself) and refuses a public key that is not an uncompressed ECDSA P-256 point. It
/// needs no agent identity, so it runs before the token lookup.
pub fn validate_enrollment_csr(csr_der: &[u8]) -> Result<(), CaError> {
    let csr =
        CertificateSigningRequestParams::from_der(&CertificateSigningRequestDer::from(csr_der))
            .map_err(CaError::BadCsr)?;
    if !matches!(csr.params.is_ca, IsCa::NoCa) {
        return Err(CaError::CsrCaFlag);
    }
    if !csr.params.subject_alt_names.is_empty() {
        return Err(CaError::CsrExtraSan);
    }
    validate_spki_p256(csr_der)
}

/// The raw public key of a certificate signing request. A rotation compares it with the key of the
/// current certificate (FR-ID-03).
pub fn csr_public_key(csr_der: &[u8]) -> Result<Vec<u8>, CaError> {
    use x509_parser::certification_request::X509CertificationRequest;
    use x509_parser::prelude::FromDer;

    let (_, csr) =
        X509CertificationRequest::from_der(csr_der).map_err(|_| CaError::CsrKeyAlgorithm)?;
    Ok(csr
        .certification_request_info
        .subject_pki
        .subject_public_key
        .data
        .to_vec())
}

/// Refuses a public key that is not an uncompressed ECDSA P-256 point (ADR-0026).
///
/// The check reads the curve from the SubjectPublicKeyInfo. `rcgen` names the key algorithm after
/// the signature algorithm of the request, so a P-384 key with an `ecdsa-with-SHA256` signature
/// would pass a check that reads the signature algorithm alone.
fn validate_spki_p256(csr_der: &[u8]) -> Result<(), CaError> {
    use x509_parser::certification_request::X509CertificationRequest;
    use x509_parser::oid_registry::{OID_EC_P256, OID_KEY_TYPE_EC_PUBLIC_KEY};
    use x509_parser::prelude::FromDer;
    use x509_parser::public_key::PublicKey;

    let (_, csr) =
        X509CertificationRequest::from_der(csr_der).map_err(|_| CaError::CsrKeyAlgorithm)?;
    let spki = &csr.certification_request_info.subject_pki;
    if spki.algorithm.algorithm != OID_KEY_TYPE_EC_PUBLIC_KEY {
        return Err(CaError::CsrKeyAlgorithm);
    }
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|parameters| parameters.as_oid().ok())
        .ok_or(CaError::CsrKeyAlgorithm)?;
    if curve != OID_EC_P256 {
        return Err(CaError::CsrKeyAlgorithm);
    }
    match spki.parsed() {
        Ok(PublicKey::EC(point))
            if point.data().first() == Some(&0x04) && point.data().len() == 65 =>
        {
            Ok(())
        }
        _ => Err(CaError::CsrKeyAlgorithm),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fr_id_03_a_leaf_starts_five_minutes_early_but_not_before_the_issuer() {
        use super::{AGENT_CERT_LIFETIME, leaf_not_before};
        use time::{Duration, OffsetDateTime};

        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(100);
        let long_ago = now - Duration::days(30);
        assert_eq!(
            leaf_not_before(now, AGENT_CERT_LIFETIME, long_ago),
            now - Duration::minutes(5)
        );
        let issuer_start = now - Duration::minutes(2);
        assert_eq!(
            leaf_not_before(now, AGENT_CERT_LIFETIME, issuer_start),
            issuer_start
        );
        // A short test lifetime shortens the backdate.
        assert_eq!(
            leaf_not_before(now, Duration::seconds(8), long_ago),
            now - Duration::seconds(2)
        );
    }

    use uuid::Uuid;

    use super::{
        AGENT_CERT_LIFETIME, ISSUING_CA_LIFETIME, ROOT_CA_LIFETIME, agent_san_uri, random_serial,
    };

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

    #[test]
    fn a_random_serial_is_16_bytes_with_the_top_bit_clear() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let serial = random_serial().expect("the generator must work");
            let bytes = serial.to_bytes();
            assert_eq!(bytes.len(), 16);
            assert_eq!(bytes[0] & 0x80, 0, "the top bit must be clear");
            assert!(bytes.iter().any(|byte| *byte != 0), "the serial is zero");
            assert!(seen.insert(bytes), "a serial repeated");
        }
    }
}
