//! Trasforma (file scansionati + proposta AI) in un piano di spostamenti
//! concreto, risolvendo i conflitti di nome prima di toccare il disco.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ai_analyzer::{sanitize_destination, OrganizationProposal};
use crate::scanner::FileInfo;

/// Limite storico di Windows: oltre questo un path puo' non essere apribile.
const MAX_PATH_LEN: usize = 260;
const DEFAULT_FOLDER: &str = "Vari";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Move {
    pub source: String,
    pub destination: String,
    pub relative_destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub destination: String,
    pub resolved_as: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingStatistics {
    pub total_moves: usize,
    pub new_folders: usize,
    pub unmatched: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingPlan {
    pub root: String,
    pub moves: Vec<Move>,
    pub conflicts: Vec<Conflict>,
    pub warnings: Vec<String>,
    pub statistics: MappingStatistics,
}

/// Glob minimale: `*.ext`, `prefisso*`, `*suffisso`, `*parte*`, nome esatto.
fn matches_pattern(pattern: &str, name_lower: &str) -> bool {
    let p = pattern.trim();
    if p == "*" {
        return true;
    }
    match (p.starts_with('*'), p.ends_with('*')) {
        (true, true) if p.len() > 2 => name_lower.contains(&p[1..p.len() - 1]),
        (true, false) => name_lower.ends_with(&p[1..]),
        (false, true) => name_lower.starts_with(&p[..p.len() - 1]),
        _ => name_lower == p,
    }
}

/// Aggiunge un suffisso prima dell'estensione: "foto.jpg" -> "foto_1.jpg".
fn with_suffix(name: &str, n: usize) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}_{n}.{ext}"),
        _ => format!("{name}_{n}"),
    }
}

pub fn create_mapping_plan(
    root: &str,
    files: &[FileInfo],
    proposal: &OrganizationProposal,
) -> MappingPlan {
    let root_path = Path::new(root);
    let mut moves = Vec::new();
    let mut conflicts = Vec::new();
    let mut warnings = Vec::new();
    let mut folders: HashSet<String> = HashSet::new();
    // Chiavi lowercase: su Windows il file system e' case-insensitive.
    let mut taken: HashSet<String> = HashSet::new();
    let mut unmatched = 0usize;
    let mut skipped_in_place = 0usize;

    for file in files {
        let name_lower = file.name.to_lowercase();

        // Prima regola che matcha vince: l'ordine dell'AI e' significativo.
        let folder = proposal
            .rules
            .iter()
            .find(|r| matches_pattern(&r.pattern, &name_lower))
            .and_then(|r| sanitize_destination(&r.destination))
            .unwrap_or_else(|| {
                unmatched += 1;
                DEFAULT_FOLDER.to_string()
            });

        // Risoluzione conflitti: due file diversi non possono finire sullo stesso path.
        let mut relative = format!("{}/{}", folder, file.name);
        if taken.contains(&relative.to_lowercase()) {
            let mut n = 1;
            loop {
                let candidate_name = with_suffix(&file.name, n);
                let candidate = format!("{folder}/{candidate_name}");
                if !taken.contains(&candidate.to_lowercase()) {
                    conflicts.push(Conflict {
                        destination: relative.clone(),
                        resolved_as: candidate.clone(),
                        reason: format!("Nome gia' usato da un altro file in {folder}"),
                    });
                    relative = candidate;
                    break;
                }
                n += 1;
            }
        }
        taken.insert(relative.to_lowercase());

        let absolute: PathBuf = root_path.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        let absolute_str = absolute.to_string_lossy().to_string();

        // Il file e' gia' al posto giusto: nessuna operazione da fare.
        if absolute_str.to_lowercase() == file.path.to_lowercase() {
            skipped_in_place += 1;
            folders.insert(folder);
            continue;
        }

        if absolute_str.chars().count() > MAX_PATH_LEN {
            warnings.push(format!(
                "Percorso lungo {} caratteri (limite {MAX_PATH_LEN}): {relative}",
                absolute_str.chars().count()
            ));
        }

        folders.insert(folder);
        moves.push(Move {
            source: file.path.clone(),
            destination: absolute_str,
            relative_destination: relative,
        });
    }

    if skipped_in_place > 0 {
        warnings.push(format!("{skipped_in_place} file sono gia' nella posizione corretta e verranno saltati."));
    }
    if !conflicts.is_empty() {
        warnings.push(format!("{} nomi duplicati rinominati con suffisso _N.", conflicts.len()));
    }

    // Conta anche le cartelle intermedie ("A/B" implica "A").
    let mut all_folders: HashSet<String> = HashSet::new();
    for f in &folders {
        let mut acc = String::new();
        for part in f.split('/') {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            all_folders.insert(acc.clone());
        }
    }

    let statistics = MappingStatistics {
        total_moves: moves.len(),
        new_folders: all_folders.len(),
        unmatched,
    };

    MappingPlan { root: root.to_string(), moves, conflicts, warnings, statistics }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_variants() {
        assert!(matches_pattern("*.pdf", "report.pdf"));
        assert!(!matches_pattern("*.pdf", "report.doc"));
        assert!(matches_pattern("img_*", "img_001.png"));
        assert!(matches_pattern("*fattura*", "2024_fattura_ac.pdf"));
        assert!(matches_pattern("readme.md", "readme.md"));
    }

    #[test]
    fn suffix_keeps_extension() {
        assert_eq!(with_suffix("foto.jpg", 2), "foto_2.jpg");
        assert_eq!(with_suffix("LICENSE", 1), "LICENSE_1");
    }
}
