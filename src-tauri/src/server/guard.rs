//! Confinamento dei percorsi.
//!
//! Ogni operazione che arriva dalla rete deve restare dentro le cartelle che
//! l'utente ha messo in `monitored_folders` dalla finestra desktop. Questo e'
//! il controllo che regge tutto: anche con una sessione valida in mano, una
//! richiesta che nomina `C:\Windows` viene rifiutata qui, prima di toccare il
//! disco.
//!
//! L'allowlist si modifica solo dal desktop: `set_config` via HTTP rifiuta di
//! cambiarla (vedi `api::set_config`), altrimenti basterebbe una richiesta per
//! allargare il recinto e poi lavorare indisturbati dentro il nuovo confine.

use std::path::{Component, Path, PathBuf};

/// Nomi che su Windows non sono file ma dispositivi: aprirli ha effetti
/// collaterali (blocchi, letture da porte seriali) anche solo in lettura.
const DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Le radici consentite, risolte. Una cartella configurata ma non piu'
/// esistente sparisce dalla lista: non si puo' autorizzare cio' che non c'e'.
pub fn roots() -> Vec<PathBuf> {
    crate::storage::load_config()
        .monitored_folders
        .iter()
        .filter_map(|f| std::fs::canonicalize(Path::new(f)).ok())
        .collect()
}

/// Risolve `path` e garantisce che stia sotto una delle radici consentite.
///
/// Funziona anche su percorsi che non esistono ancora (le cartelle di
/// destinazione di un piano): in quel caso risolve l'antenato piu' vicino che
/// esiste e ci riattacca il resto. E' quell'antenato a essere verificato, cosi'
/// un link simbolico che punta fuori dal recinto non puo' essere usato come
/// scorciatoia: `canonicalize` lo segue e il confronto fallisce.
pub fn ensure_within(path: &str) -> Result<PathBuf, String> {
    let roots = roots();
    if roots.is_empty() {
        return Err(
            "Nessuna cartella autorizzata: aprine una dalla finestra di LOGICA sul PC prima di \
             lavorare da remoto."
                .to_string(),
        );
    }
    let resolved = resolve(path)?;
    if roots.iter().any(|r| resolved.starts_with(r)) {
        Ok(resolved)
    } else {
        Err(format!(
            "Percorso fuori dalle cartelle autorizzate: {path}. Da remoto si puo' lavorare solo \
             dentro le cartelle scelte sul PC."
        ))
    }
}

/// Come `ensure_within` ma per una lista: basta un percorso fuori recinto e
/// l'intera operazione viene rifiutata. Meglio non fare niente che fare meta'
/// di uno spostamento.
pub fn ensure_all_within(paths: &[String]) -> Result<(), String> {
    for p in paths {
        ensure_within(p)?;
    }
    Ok(())
}

/// Risoluzione in forma canonica, senza richiedere che il file esista.
fn resolve(path: &str) -> Result<PathBuf, String> {
    let raw = Path::new(path);
    if raw.as_os_str().is_empty() {
        return Err("Percorso vuoto.".to_string());
    }
    if !raw.is_absolute() {
        return Err(format!("Percorso non assoluto: {path}"));
    }
    reject_traversal(raw)?;
    reject_device_names(raw)?;

    // Dal percorso completo si risale finche' non si trova qualcosa che esiste,
    // poi si ridiscende riattaccando i pezzi mancanti.
    let mut missing: Vec<&std::ffi::OsStr> = Vec::new();
    let mut cursor = raw;
    loop {
        match std::fs::canonicalize(cursor) {
            Ok(base) => {
                let mut full = base;
                for part in missing.iter().rev() {
                    full.push(part);
                }
                return Ok(full);
            }
            Err(_) => match (cursor.parent(), cursor.file_name()) {
                (Some(parent), Some(name)) => {
                    missing.push(name);
                    cursor = parent;
                }
                // Risaliti fino alla radice senza trovare niente di esistente:
                // il disco stesso non c'e'.
                _ => return Err(format!("Percorso non risolvibile: {path}")),
            },
        }
    }
}

/// `..` non serve mai a un percorso legittimo che arriva dalla rete, e permette
/// di uscire dal recinto prima ancora che il confronto avvenga.
///
/// Il controllo lavora sui componenti e non sul testo: la lettera di unita' e
/// il prefisso `\\?\` dei percorsi canonici di Windows contengono anche loro
/// dei due punti, e cercarli nella stringa intera farebbe rifiutare percorsi
/// legittimi. Come componenti, invece, restano fuori dal confronto.
fn reject_traversal(path: &Path) -> Result<(), String> {
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err("Il percorso contiene '..': rifiutato.".to_string())
            }
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                // Dentro un percorso verbatim (`\\?\C:\...`) Windows non
                // attribuisce nessun significato a `..`, e Rust lo restituisce
                // come componente normale: il ramo sopra non lo vedrebbe.
                if text == ".." {
                    return Err("Il percorso contiene '..': rifiutato.".to_string());
                }
                // Gli stream alternativi NTFS (`nota.txt:nascosto`) nascondono
                // contenuto dietro il nome di un file lecito.
                if text.contains(':') {
                    return Err("Il percorso contiene ':': rifiutato.".to_string());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn reject_device_names(path: &Path) -> Result<(), String> {
    for c in path.components() {
        if let Component::Normal(part) = c {
            let name = part.to_string_lossy();
            let stem = name.split('.').next().unwrap_or(&name).to_ascii_uppercase();
            if DEVICE_NAMES.contains(&stem.as_str()) {
                return Err(format!("Nome di dispositivo riservato nel percorso: {name}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::storage::tests::TempHome;

    /// Home finta con una sola cartella autorizzata, gia' esistente.
    fn recinto(tag: &str) -> (TempHome, PathBuf) {
        let home = TempHome::new(tag);
        crate::storage::ensure_layout().expect("layout");

        let dir = std::env::temp_dir().join(format!("logica_recinto_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cartella autorizzata");

        let mut config = crate::storage::load_config();
        config.monitored_folders = vec![dir.to_string_lossy().to_string()];
        crate::storage::save_config(&config).expect("config");

        let risolta = std::fs::canonicalize(&dir).expect("canonica");
        (home, risolta)
    }

    #[test]
    fn dentro_si_passa_fuori_no() {
        let (_home, dir) = recinto("dentro_fuori");

        let dentro = dir.join("documento.pdf");
        assert!(ensure_within(&dentro.to_string_lossy()).is_ok());

        // Il file di sistema piu' innocuo che esista: comunque fuori.
        assert!(ensure_within("C:/Windows/System32/drivers/etc/hosts").is_err());
    }

    /// La destinazione di un piano non esiste ancora quando la si controlla:
    /// deve valere lo stesso il confine dell'antenato piu' vicino.
    #[test]
    fn una_destinazione_non_ancora_creata_e_ammessa_se_dentro() {
        let (_home, dir) = recinto("destinazione");

        let futura = dir.join("Documenti").join("2026").join("fattura.pdf");
        assert!(ensure_within(&futura.to_string_lossy()).is_ok());

        let futura_fuori = dir.join("..").join("altrove").join("fattura.pdf");
        assert!(ensure_within(&futura_fuori.to_string_lossy()).is_err());
    }

    /// I `..` vanno costruiti nel testo, non con `join`: `PathBuf` li collassa
    /// da solo sui percorsi verbatim, ed e' proprio la forma testuale quella
    /// che arriverebbe dalla rete.
    #[test]
    fn risalire_con_puntini_non_serve() {
        let (_home, dir) = recinto("puntini");

        let verbatim = format!("{}/../../segreti.txt", dir.display());
        let errore = ensure_within(&verbatim).expect_err("doveva fallire");
        assert!(errore.contains(".."), "errore inatteso: {errore}");

        // Stessa cosa nella forma comune, senza il prefisso `\\?\`.
        let semplice = format!(
            "{}/../../segreti.txt",
            dir.display().to_string().trim_start_matches(r"\\?\")
        );
        let errore = ensure_within(&semplice).expect_err("doveva fallire");
        assert!(errore.contains(".."), "errore inatteso: {errore}");
    }

    #[test]
    fn i_nomi_di_dispositivo_sono_rifiutati() {
        let (_home, dir) = recinto("dispositivi");

        for nome in ["NUL", "con.txt", "COM1"] {
            let path = dir.join(nome);
            assert!(
                ensure_within(&path.to_string_lossy()).is_err(),
                "{nome} doveva essere rifiutato"
            );
        }
    }

    /// Gli stream alternativi NTFS nascondono contenuto dietro un file lecito.
    #[test]
    fn gli_stream_alternativi_sono_rifiutati() {
        let (_home, dir) = recinto("stream");

        let subdolo = format!("{}:nascosto", dir.join("nota.txt").to_string_lossy());
        assert!(ensure_within(&subdolo).is_err());
    }

    #[test]
    fn senza_cartelle_autorizzate_non_si_tocca_niente() {
        let _home = TempHome::new("nessun_recinto");
        crate::storage::ensure_layout().expect("layout");

        let errore = ensure_within("C:\\").expect_err("doveva fallire");
        assert!(errore.contains("Nessuna cartella autorizzata"), "errore inatteso: {errore}");
    }

    #[test]
    fn una_lista_con_un_solo_percorso_fuori_viene_respinta_tutta() {
        let (_home, dir) = recinto("lista");

        let buono = dir.join("a.txt").to_string_lossy().to_string();
        let cattivo = "C:/Windows/notepad.exe".to_string();
        assert!(ensure_all_within(std::slice::from_ref(&buono)).is_ok());
        assert!(ensure_all_within(&[buono, cattivo]).is_err());
    }
}
