use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

fn config_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".config").join("devx"))
}

/// Load or generate the local CA keypair and certificate.
/// Returns (CA certificate PEM, CA key PEM) as strings.
pub fn ensure_ca() -> Result<(String, String)> {
    let dir = config_dir()?;
    let ca_key_path = dir.join("ca.key");
    let ca_cert_path = dir.join("ca.pem");

    if ca_key_path.exists() && ca_cert_path.exists() {
        let key_pem = std::fs::read_to_string(&ca_key_path).context("failed to read ca.key")?;
        let cert_pem = std::fs::read_to_string(&ca_cert_path).context("failed to read ca.pem")?;
        return Ok((cert_pem, key_pem));
    }

    std::fs::create_dir_all(&dir).context("failed to create ~/.config/devx")?;

    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .context("failed to generate CA key")?;

    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, "devx local CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "devx");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = OffsetDateTime::now_utc() + time::Duration::days(3650);

    let ca_cert = params
        .self_signed(&key_pair)
        .context("failed to self-sign CA cert")?;

    let cert_pem = ca_cert.pem();
    let key_pem = key_pair.serialize_pem();

    std::fs::write(&ca_key_path, &key_pem).context("failed to write ca.key")?;
    std::fs::write(&ca_cert_path, &cert_pem).context("failed to write ca.pem")?;

    eprintln!("Generated local CA at ~/.config/devx/ca.pem");
    eprintln!(
        "Run: sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain ~/.config/devx/ca.pem"
    );

    Ok((cert_pem, key_pem))
}

/// Generate a leaf certificate signed by the CA for the given domains.
/// Returns a rustls ServerConfig ready to use.
pub fn generate_server_config(
    ca_cert_pem: &str,
    ca_key_pem: &str,
    domains: &[String],
) -> Result<Arc<rustls::ServerConfig>> {
    let ca_key_pair = KeyPair::from_pem(ca_key_pem).context("failed to parse CA key")?;
    let ca_params = CertificateParams::from_ca_cert_pem(ca_cert_pem)
        .context("failed to parse CA cert params")?;
    let ca_cert = ca_params
        .self_signed(&ca_key_pair)
        .context("failed to reconstruct CA cert")?;

    let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .context("failed to generate leaf key")?;

    let mut sans: Vec<SanType> = domains
        .iter()
        .map(|d| SanType::DnsName(d.clone().try_into().unwrap()))
        .collect();

    // Always include localhost and 127.0.0.1
    if !domains.iter().any(|d| d == "localhost") {
        sans.push(SanType::DnsName(
            "localhost".to_string().try_into().unwrap(),
        ));
    }
    sans.push(SanType::IpAddress(std::net::IpAddr::V4(
        std::net::Ipv4Addr::new(127, 0, 0, 1),
    )));

    let mut leaf_params =
        CertificateParams::new(domains.iter().map(|d| d.to_string()).collect::<Vec<_>>())
            .context("failed to create leaf params")?;

    leaf_params.subject_alt_names = sans;
    leaf_params.not_before = OffsetDateTime::now_utc();
    leaf_params.not_after = OffsetDateTime::now_utc() + time::Duration::days(365);
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "devx local");

    let leaf_cert = leaf_params
        .signed_by(&leaf_key, &ca_cert, &ca_key_pair)
        .context("failed to sign leaf cert")?;

    let cert_chain = vec![
        CertificateDer::from(leaf_cert.der().to_vec()),
        CertificateDer::from(ca_cert.der().to_vec()),
    ];
    let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)
        .context("failed to build rustls ServerConfig")?;

    Ok(Arc::new(server_config))
}

/// Trust the CA certificate in the system keychain.
pub fn trust_ca() -> Result<()> {
    let dir = config_dir()?;
    let ca_cert_path = dir.join("ca.pem");

    if !ca_cert_path.exists() {
        // Generate it first
        ensure_ca()?;
    }

    let ca_path_str = ca_cert_path.to_string_lossy().to_string();

    if cfg!(target_os = "macos") {
        eprintln!("Adding CA to macOS System Keychain (requires sudo)...");
        let status = std::process::Command::new("sudo")
            .args([
                "security",
                "add-trusted-cert",
                "-d",
                "-r",
                "trustRoot",
                "-k",
                "/Library/Keychains/System.keychain",
                &ca_path_str,
            ])
            .status()
            .context("failed to run security command")?;

        if status.success() {
            eprintln!("CA trusted successfully");
        } else {
            anyhow::bail!("security add-trusted-cert failed (exit {})", status);
        }
    } else {
        // Linux
        let dest = PathBuf::from("/usr/local/share/ca-certificates/devx-ca.crt");
        eprintln!("Copying CA to {} (requires sudo)...", dest.display());

        let status = std::process::Command::new("sudo")
            .args(["cp", &ca_path_str, &dest.to_string_lossy()])
            .status()
            .context("failed to copy CA cert")?;

        if !status.success() {
            anyhow::bail!("failed to copy CA cert");
        }

        let status = std::process::Command::new("sudo")
            .args(["update-ca-certificates"])
            .status()
            .context("failed to run update-ca-certificates")?;

        if status.success() {
            eprintln!("CA trusted successfully");
        } else {
            anyhow::bail!("update-ca-certificates failed (exit {})", status);
        }
    }

    Ok(())
}
