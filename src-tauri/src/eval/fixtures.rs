//! I documenti su cui gira la valutazione.
//!
//! Sono scritti qui e non presi dal disco dell'utente per tre motivi. Il primo
//! e' che una misura deve poter essere rifatta: se il corpus cambia fra due
//! esecuzioni, i numeri non si possono confrontare. Il secondo e' che per
//! misurare le risposte inventate bisogna sapere **cosa non c'e'**, e questo si
//! sa solo se il corpus l'hai scritto tu. Il terzo e' che i documenti veri di
//! chi usa il programma non devono finire dentro un test.
//!
//! Ogni documento contiene almeno un dato controllabile alla lettera — un
//! codice, un importo, una data — e i dati sono scelti perche' non si possano
//! indovinare: un modello che risponde "1.240,00" senza aver aperto il file
//! l'ha letto davvero.

use std::path::{Path, PathBuf};

/// Un file del corpus: dove sta, dentro la cartella, e cosa contiene.
struct Doc {
    path: &'static str,
    body: &'static str,
}

const DOCS: &[Doc] = &[
    Doc {
        path: "fatture/fattura_2024_017.txt",
        body: "FATTURA N. 2024/017
Emessa il 12 novembre 2024
Cliente: Rossi Impianti S.r.l., via Garibaldi 8, Vicenza
Partita IVA: 03918470249

Descrizione: sostituzione quadro elettrico e messa a norma impianto.
Imponibile: 1.016,39 euro
IVA 22%: 223,61 euro
Totale: 1.240,00 euro

Pagamento a 30 giorni, bonifico su IBAN IT60X0542811101000000123456.
Scadenza: 12 dicembre 2024.",
    },
    Doc {
        path: "casa/caldaia_manutenzione.txt",
        body: "Manutenzione caldaia - appartamento di via Piave

Modello: Vaillant ecoTEC plus VMW 246
Matricola: 21-4471-093

Ultimo intervento: 4 marzo 2025, eseguito da Termoidraulica Bassan.
Operazioni: analisi dei fumi, pulizia dello scambiatore, verifica della
pressione del circuito (riportata a 1,4 bar).
Costo: 118 euro, ricambi esclusi.

Il prossimo controllo va fatto entro il 4 marzo 2027: il bollino ha validita'
biennale. Il libretto di impianto e' nel cassetto della cucina.",
    },
    Doc {
        path: "casa/contratto_affitto.txt",
        body: "Contratto di locazione - via Piave 22, interno 5

Decorrenza: 1 settembre 2023, durata 4 anni piu' 4.
Canone mensile: 650 euro, da versare entro il giorno 5 di ogni mese.
Deposito cauzionale: 1.300 euro, pari a due mensilita'.
Spese condominiali: 85 euro al mese, a carico del conduttore.

Disdetta: va comunicata con raccomandata almeno sei mesi prima della
scadenza. Gli animali domestici sono ammessi.
Le piccole riparazioni sono a carico dell'inquilino; quelle straordinarie
restano al proprietario.",
    },
    Doc {
        path: "lavoro/verbale_riunione_marzo.txt",
        body: "Verbale della riunione del 18 marzo 2025

Presenti: Elena Barbieri, Marco Dellai, Sofia Nguyen.
Assente giustificato: Paolo Zanetti.

Decisioni prese:
- Il rilascio della versione 2.4 slitta al 30 aprile, per lasciare tempo alle
  prove di carico.
- La migrazione del database viene affidata a Marco Dellai.
- Si abbandona il fornitore attuale per l'invio delle email: la scelta cade su
  un servizio interno, da valutare entro maggio.

Rimane aperto: chi si occupa della documentazione per i clienti.",
    },
    Doc {
        path: "lavoro/progetto_atlas.md",
        body: "# Progetto Atlas

Sistema di catalogazione per l'archivio storico comunale.

## Stato
Prima consegna prevista per il 15 giugno 2025. Il committente e' il Comune di
Schio, referente l'architetto Ferrari.

## Scelte tecniche
- Backend in Rust, con PostgreSQL 16.
- Le immagini digitalizzate restano su disco, nel database vanno solo i
  percorsi: sono circa 400 gigabyte e in tabella non ci stanno bene.
- Autenticazione delegata allo SPID del Comune.

## Da fare
La ricerca a testo pieno sui documenti scansionati e' rimandata alla seconda
consegna: prima serve capire che qualita' ha il riconoscimento del testo.",
    },
    Doc {
        path: "viaggi/prenotazione_lisbona.txt",
        body: "Viaggio a Lisbona

Volo di andata: 4 giugno 2025, Venezia-Lisbona, partenza alle 7:15,
arrivo alle 9:40. Compagnia TAP, volo TP826.
Codice di prenotazione: QK7T2M.
Bagaglio a mano incluso, bagaglio in stiva no.

Ritorno: 11 giugno, partenza alle 20:05.

Alloggio: Casa do Bairro, rua da Rosa 145, camera doppia con colazione,
quattro notti, 512 euro in tutto. Check-in dalle 15.",
    },
    Doc {
        path: "ricette/torta_di_mele.txt",
        body: "Torta di mele della nonna

Ingredienti:
- 3 uova
- 200 grammi di farina 00
- 150 grammi di zucchero
- mezzo litro di latte
- 4 mele renette
- una bustina di lievito
- la scorza di un limone

Sbattere le uova con lo zucchero, unire la farina setacciata con il lievito,
poi il latte a filo. Aggiungere tre mele a fette sottili e mescolare.
Versare in uno stampo da 24 centimetri, coprire con la quarta mela a
raggiera e infornare a 180 gradi per 40 minuti, forno statico.",
    },
    Doc {
        path: "auto/tagliando_2025.txt",
        body: "Tagliando auto - Fiat Panda, targa GH482KR

Eseguito il 9 gennaio 2025 presso Autofficina Menegatti, Thiene.
Chilometraggio: 87.430 km.

Interventi: cambio olio e filtro, filtro abitacolo, controllo freni.
Le pastiglie anteriori sono al 40 per cento: da sostituire al prossimo
tagliando.
Costo: 212 euro.

Prossima scadenza: 9 gennaio 2026 oppure 102.000 km, quello che viene prima.
La revisione scade a giugno 2026.",
    },
    Doc {
        path: "documenti/garanzia_frigorifero.txt",
        body: "Garanzia frigorifero

Modello: Bosch KGN39VLEB
Numero di serie: FD9812-004473
Acquistato il 3 ottobre 2024 da Elettrodomestici Fabbris, scontrino n. 4471.
Prezzo: 649 euro.

La garanzia dura 24 mesi dalla data di acquisto e copre i difetti di
fabbricazione. Non copre i danni da trasporto ne' l'usura delle guarnizioni.
Per l'assistenza serve lo scontrino: e' nella busta insieme al libretto.",
    },
    Doc {
        path: "salute/analisi_settembre.txt",
        body: "Analisi del sangue - 22 settembre 2024

Emoglobina: 14,2 g/dl
Glicemia a digiuno: 96 mg/dl
Colesterolo totale: 214 mg/dl (sopra il valore di riferimento, 200)
Colesterolo HDL: 58 mg/dl
Trigliceridi: 88 mg/dl
Ferritina: 61 ng/ml

Il medico ha consigliato di ripetere il controllo del colesterolo fra sei
mesi, senza terapia per ora, riducendo i formaggi stagionati.",
    },
];

/// Un file che serve come allegato e che sta **fuori** dalle cartelle
/// autorizzate: gli allegati si aprono perche' l'utente li sceglie, non perche'
/// il modello ci arrivi da solo. Tenerlo fuori distingue le due strade.
const ATTACHMENT: Doc = Doc {
    path: "preventivo_serramenti.txt",
    body: "Preventivo n. 88/2025 - Serramenti Dal Molin

Fornitura e posa di 6 finestre in PVC, doppio vetro basso emissivo.
Imponibile: 7.350 euro
Sconto praticato: 8 per cento
Totale con IVA al 10 per cento: 7.440,30 euro

Tempi di consegna: 45 giorni dalla conferma.
Il preventivo resta valido 60 giorni dalla data di emissione, 3 marzo 2025.
La rimozione dei vecchi serramenti e lo smaltimento sono compresi.",
};

/// Dove finiscono i documenti da indicizzare e da leggere.
pub fn corpus_dir() -> PathBuf {
    std::env::temp_dir().join("logica_eval_corpus")
}

/// Dove finisce il file da allegare, fuori dalle cartelle autorizzate.
pub fn attachment_path() -> PathBuf {
    std::env::temp_dir().join("logica_eval_allegati").join(ATTACHMENT.path)
}

/// Scrive il corpus da zero. Da zero e non "se manca": un corpus avanzato da
/// un'esecuzione precedente, magari di una versione diversa, falserebbe il
/// confronto senza farlo vedere.
pub fn write_all() -> std::io::Result<PathBuf> {
    let root = corpus_dir();
    let _ = std::fs::remove_dir_all(&root);
    for doc in DOCS {
        write_doc(&root, doc.path, doc.body)?;
    }

    let attach = attachment_path();
    let _ = std::fs::remove_dir_all(attach.parent().unwrap());
    write_doc(attach.parent().unwrap(), ATTACHMENT.path, ATTACHMENT.body)?;

    Ok(root)
}

fn write_doc(root: &Path, relative: &str, body: &str) -> std::io::Result<()> {
    let file = root.join(relative);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, body)
}

pub fn clean() {
    let _ = std::fs::remove_dir_all(corpus_dir());
    if let Some(parent) = attachment_path().parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

/// Il contenuto dell'allegato, come lo vedrebbe il modello: la finestra della
/// chat incolla il testo del file sotto al messaggio, e qui si fa lo stesso.
pub fn attachment_block() -> String {
    format!("--- allegato: {} ---\n{}", ATTACHMENT.path, ATTACHMENT.body)
}
