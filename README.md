# File Organizer AI

App desktop (Tauri 2 + React + Rust) che analizza una cartella e propone una struttura ordinata
usando **Ollama in locale** (default, offline e gratis) oppure la **Claude API** (opzionale, premium).

Niente viene spostato senza anteprima e conferma. Ogni applicazione è annullabile (undo) e, se
qualcosa fallisce a metà, il piano viene ripristinato per intero (rollback automatico).

---

## Requisiti

| Cosa | Versione | Note |
|---|---|---|
| Node.js | ≥ 18 | già presente |
| Rust | stable (rustup) | **necessario per compilare il backend** |
| Ollama | qualsiasi | opzionale ma consigliato: senza, si usa lo schema di fallback |
| Build tools Windows | MSVC + WebView2 | inclusi in Visual Studio Build Tools |

### Installare Rust (Windows)

```powershell
winget install --id Rustlang.Rustup -e
# riavvia il terminale, poi:
rustup default stable
```

Serve anche il toolchain MSVC (`winget install Microsoft.VisualStudio.2022.BuildTools`, workload
"Desktop development with C++"). WebView2 è già presente su Windows 11.

### Installare e preparare Ollama

```powershell
winget install --id Ollama.Ollama -e
ollama pull mistral        # modello di default dell'app
ollama serve               # di norma parte da solo come servizio
```

Verifica: `curl http://localhost:11434/api/tags` deve rispondere JSON.
Con una RTX 4070 Super vanno bene anche `llama3.1:8b` o `qwen2.5:7b` — il nome del modello si
cambia da **Impostazioni** dentro l'app.

---

## Avvio

```bash
npm install          # già fatto
npm run tauri dev    # dev server + finestra desktop
npm run tauri build  # eseguibile + installer in src-tauri/target/release
```

Solo frontend (senza Rust): `npm run dev` → http://localhost:5173 (le `invoke` falliranno, è
normale: i comandi vivono nel backend).

---

## Claude API (opzionale)

Imposta la chiave **prima** di avviare l'app, poi attiva "Usa Claude API" nelle impostazioni:

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."
npm run tauri dev
```

Il modello usato è `claude-opus-5`. Senza chiave l'app resta su Ollama.

---

## Come funziona

```
SELECT_DIR → SCANNING → ANALYZING → PREVIEW → APPLYING → RESULT
```

1. **SCANNING** — `walkdir` ricorsivo, salta `.git`, `node_modules`, `__pycache__`, cartelle
   nascoste e di sistema. Progresso via evento `scan-progress`. Timeout 5 minuti.
2. **ANALYZING** — statistiche + campione di 100 file → prompt → Ollama (`/api/generate`,
   `format: json`) o Claude (`/v1/messages`). Timeout 30 s. Se la risposta non è JSON valido o
   l'AI non risponde, si usa lo **schema di fallback** per estensione.
3. **PREVIEW** — albero cartelle (rinomina con doppio click, unione con drag & drop, eliminazione)
   + tabella dei primi 50 file, statistiche, conflitti e avvisi. Ogni modifica ricalcola il piano.
4. **APPLYING** — pre-flight (destinazioni dentro la radice, file ancora esistenti), `mkdir`,
   `fs::rename` uno per uno con fallback copia+rimozione tra volumi diversi. Errore o annullamento
   → rollback completo.
5. **RESULT** — riepilogo, "Apri cartella", "Annulla tutto", "Nuovo ciclo".

Scorciatoie: **Invio** = Applica, **Esc** = Indietro, **Ctrl+Z** = Undo.

---

## Struttura

```
file-organizer-ai/
├── src/
│   ├── components/
│   │   ├── StepSelector.tsx     scelta cartella, stato Ollama, impostazioni
│   │   ├── ScanningPhase.tsx    conteggio file + velocità
│   │   ├── AnalysisPhase.tsx    spinner "sta analizzando…"
│   │   ├── PreviewPhase.tsx     albero editabile + tabella mappatura
│   │   ├── ApplyPhase.tsx       barra di progresso + stima
│   │   └── ResultPhase.tsx      riepilogo finale
│   ├── App.tsx                  macchina a stati
│   ├── types.ts                 interfacce condivise col backend
│   ├── App.css                  tema scuro
│   └── main.tsx
└── src-tauri/
    ├── src/
    │   ├── main.rs              setup Tauri + comandi
    │   ├── scanner.rs           scansione file system
    │   ├── ai_analyzer.rs       prompt, Ollama, Claude, fallback, validazione
    │   ├── file_mapper.rs       regole → piano di spostamenti + conflitti
    │   └── file_operations.rs   mkdir/move/rollback/undo
    ├── capabilities/default.json
    ├── tauri.conf.json
    └── Cargo.toml
```

### Comandi Tauri

| Comando | Argomenti (JS) | Ritorna |
|---|---|---|
| `scan_directory` | `path` | `FileInfo[]` |
| `get_organization_proposal` | `files`, `useOllama`, `model`, `userPreferences` | `OrganizationProposal` |
| `create_mapping_plan` | `root`, `files`, `proposal` | `MappingPlan` |
| `apply_mapping_plan` | `plan` | `ApplyResult` |
| `cancel_apply` | — | — |
| `undo_last_operation` | — | `string` |
| `open_folder_in_explorer` | `path` | — |
| `check_ollama_connection` | — | `boolean` |

Eventi emessi: `scan-progress` (`{scanned, current}`), `apply-progress` (`{done, total, current}`).

---

## Sicurezza

- **Nessun file viene mai cancellato**: si usa solo `rename` (o copia + rimozione dell'originale
  tra volumi diversi). `remove_dir` tocca solo cartelle vuote create dall'app stessa.
- Le destinazioni sono sanificate: niente `..`, niente percorsi assoluti, niente caratteri
  illegali su Windows, massimo 4 livelli di profondità.
- Pre-flight: ogni destinazione deve restare dentro la cartella scelta.
- Conflitti di nome risolti con suffisso `_1`, `_2`, … prima di toccare il disco.
- Log di undo salvato in `.file-organizer-undo.json` dentro la cartella organizzata.

---

## Troubleshooting

**"Ollama offline"** — il servizio non risponde su `localhost:11434`. Avvia `ollama serve` e
controlla di aver scaricato il modello (`ollama list`). Senza Ollama l'app funziona lo stesso con
lo schema per estensione.

**La proposta arriva sempre dal "fallback"** — il modello non restituisce JSON valido. Prova un
modello più capace (`ollama pull llama3.1:8b`) e cambialo nelle impostazioni; oppure guarda
l'output della console dev, dove viene stampato il motivo (`[ai_analyzer] fallback attivato: …`).

**`cargo` / `rustc` non trovato** — Rust non è installato: vedi la sezione Requisiti.

**Errore MSVC / `link.exe` non trovato** — mancano i Visual Studio Build Tools con il workload
C++.

**Scansione lentissima** — hai scelto una cartella enorme (`C:\`). Punta a cartelle concrete
(Downloads, Desktop, Documenti). Le cartelle di sistema sono comunque saltate.

**"Percorso lungo N caratteri"** — la destinazione supera i 260 caratteri di Windows. Rinomina
qualche cartella nell'anteprima o abilita i long path nel registro di sistema.

**Undo non disponibile** — l'undo è in memoria e vale per l'ultima operazione della sessione. Il
file `.file-organizer-undo.json` nella cartella resta comunque come traccia degli spostamenti.

---

## Note sull'implementazione

- Progetto su **Tauri 2** (il blueprint citava le dipendenze 1.5, ma il `tauri.conf.json`
  richiesto era già in formato v2: le API v2 sono quelle attuali e supportate).
- La scansione e l'applicazione girano in `spawn_blocking`: la UI non si blocca mai.
- Test unitari Rust: `cd src-tauri && cargo test` (estrazione JSON, sanificazione path, glob,
  suffissi anti-conflitto).
