//! Certificato TLS auto-firmato.
//!
//! Serve a cifrare il traffico verso l'altro PC: senza, la password e il
//! cookie di sessione viaggerebbero in chiaro sulla rete di casa, leggibili da
//! chiunque sia collegato allo stesso Wi-Fi.
//!
//! Il certificato se lo firma da solo, quindi il browser mostra un avviso la
//! prima volta: nessuna autorita' pubblica garantisce per lui. L'avviso si
//! accetta una volta sola, dopo aver confrontato l'impronta mostrata dal
//! browser con quella scritta nelle impostazioni di LOGICA. Se le due impronte
//! non coincidono, non stai parlando con il tuo PC: non proseguire.

use std::net::IpAddr;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// Un certificato piu' longevo di cosi' non aggiunge sicurezza, e rigenerarlo
/// ogni tanto limita il danno se la chiave privata finisse copiata.
const VALIDITY_DAYS: i64 = 730;

pub struct Material {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    /// SHA-256 del certificato, in maiuscolo con i due punti: la stessa forma
    /// che mostrano i browser.
    pub fingerprint: String,
}

fn tls_dir() -> Result<PathBuf, String> {
    let dir = crate::storage::logica_dir()?.join("tls");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creazione di {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Carica il certificato dal disco, generandolo al primo avvio.
///
/// Se le impostazioni di rete cambiano (nuovo indirizzo sulla LAN) il vecchio
/// certificato resta valido: il browser si lamenta del nome, non della
/// cifratura. Cancellare `~/.logica/tls/` lo fa rigenerare.
pub fn load_or_create() -> Result<Material, String> {
    let dir = tls_dir()?;
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");

    if let (Ok(cert_pem), Ok(key_pem)) =
        (std::fs::read(&cert_path), std::fs::read(&key_path))
    {
        let fingerprint = fingerprint_of(&cert_pem)?;
        return Ok(Material { cert_pem, key_pem, fingerprint });
    }

    let (cert_pem, key_pem) = generate()?;
    std::fs::write(&cert_path, &cert_pem).map_err(|e| format!("scrittura del certificato: {e}"))?;
    std::fs::write(&key_path, &key_pem).map_err(|e| format!("scrittura della chiave: {e}"))?;
    restrict_to_owner(&key_path);

    let fingerprint = fingerprint_of(&cert_pem)?;
    Ok(Material { cert_pem, key_pem, fingerprint })
}

/// Butta via il materiale esistente: al prossimo avvio del server ne nasce uno
/// nuovo. Da usare se sospetti che la chiave privata sia stata copiata.
pub fn regenerate() -> Result<Material, String> {
    let dir = tls_dir()?;
    let _ = std::fs::remove_file(dir.join("cert.pem"));
    let _ = std::fs::remove_file(dir.join("key.pem"));
    load_or_create()
}

fn generate() -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    if let Some(host) = hostname() {
        names.push(host.clone());
        // Windows risolve i nomi della LAN anche con il suffisso .local.
        names.push(format!("{host}.local"));
    }
    if let Some(ip) = lan_address() {
        names.push(ip.to_string());
    }

    let mut params =
        rcgen::CertificateParams::new(names).map_err(|e| format!("parametri del certificato: {e}"))?;

    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, "LOGICA");
    dn.push(rcgen::DnType::OrganizationName, "LOGICA (certificato locale)");
    params.distinguished_name = dn;

    let now = std::time::SystemTime::now();
    params.not_before = now.into();
    params.not_after = (now + std::time::Duration::from_secs(VALIDITY_DAYS as u64 * 86_400)).into();

    let key = rcgen::KeyPair::generate().map_err(|e| format!("generazione della chiave: {e}"))?;
    let cert = params.self_signed(&key).map_err(|e| format!("firma del certificato: {e}"))?;

    Ok((cert.pem().into_bytes(), key.serialize_pem().into_bytes()))
}

/// Impronta del certificato, calcolata sul DER estratto dal PEM.
fn fingerprint_of(cert_pem: &[u8]) -> Result<String, String> {
    let text = String::from_utf8_lossy(cert_pem);
    let body: String = text
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    let der = base64_decode(&body)?;
    let digest = Sha256::digest(&der);
    Ok(digest.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"))
}

/// Decodifica base64 senza tirarsi dietro una dipendenza per venti righe.
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for ch in input.bytes() {
        if ch == b'=' || ch.is_ascii_whitespace() {
            continue;
        }
        let value = TABLE
            .iter()
            .position(|c| *c == ch)
            .ok_or_else(|| "certificato PEM non valido".to_string())? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

fn hostname() -> Option<String> {
    std::env::var("COMPUTERNAME").ok().filter(|h| !h.is_empty()).map(|h| h.to_lowercase())
}

/// Indirizzo di questa macchina sulla rete locale.
///
/// La `connect` su UDP non manda pacchetti: sceglie solo l'interfaccia che il
/// sistema userebbe per uscire, ed e' il modo piu' corto per sapere quale
/// indirizzo vedra' l'altro PC.
pub fn lan_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

#[cfg(windows)]
fn restrict_to_owner(path: &std::path::Path) {
    let target = path.display().to_string();
    let user = std::env::var("USERNAME").unwrap_or_default();
    if user.is_empty() {
        return;
    }
    let _ = std::process::Command::new("icacls")
        .args([&target, "/inheritance:r", "/grant:r", &format!("{user}:(F)")])
        .output();
}

#[cfg(not(windows))]
fn restrict_to_owner(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn il_certificato_generato_ha_una_impronta_leggibile() {
        let (cert_pem, key_pem) = generate().expect("generazione");
        assert!(String::from_utf8_lossy(&cert_pem).starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(!key_pem.is_empty());

        let fp = fingerprint_of(&cert_pem).expect("impronta");
        // 32 byte in esadecimale separati dai due punti.
        assert_eq!(fp.split(':').count(), 32);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit() || c == ':'));
    }

    #[test]
    fn base64_decodifica_come_atteso() {
        assert_eq!(base64_decode("TE9HSUNB").expect("decode"), b"LOGICA");
    }
}
