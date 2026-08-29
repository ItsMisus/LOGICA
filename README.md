# LOGICA — organizza i file chiacchierando

App desktop (Tauri 2 + React + Rust) con un assistente conversazionale, **Logica**, che capisce a
parole quali cartelle sistemare, propone uno schema, lo applica e sa tornare indietro.

Il motore AI è **Ollama in locale** (default, offline e gratis) oppure la **Claude API**
(opzionale). Niente viene spostato senza anteprima e approvazione; ogni applicazione è annullabile.

```
┌────────────────────────────────────┬──────────────────┐
│ Logica (online · Ollama · mistral) │ [+ Nuova chat]   │
├────────────────────────────────────┼──────────────────┤
│ AI: Dimmi quale cartella sistemare │ Chat recenti:    │
│ User: organizza Downloads          │ • Organizza Do…  │
│ AI: 234 file. Per tipo, data o…?   │ • Desktop        │
│ User: per tipo                     │                  │
│ AI: [SCHEMA PROPOSTO + tabella]    │ 1.234 file       │
│     [Approva] [Modifica] [Annulla] │ questo mese      │
│ AI: ✅ 234 file spostati            │                  │
├────────────────────────────────────┼──────────────────┤
│ [📁] [messaggio…] [Invia] [↩ Undo] │ ⚙️ Impostazioni  │
└────────────────────────────────────┴──────────────────┘
```

---

## Requisiti

| Cosa | Versione | Note |
|---|---|---|
| Node.js | ≥ 18 | |
| Rust | stable (rustup) | **necessario per compilare il backend** |
| Ollama | qualsiasi | opzionale: senza, restano i criteri deterministici |
| Build tools Windows | MSVC + WebView2 | Visual Studio Build Tools, workload C++ |

```powershell
winget install --id Rustlang.Rustup -e
rustup default stable

winget install --id Ollama.Ollama -e
ollama pull mistral        # modello di default
```

Verifica Ollama: `curl http://localhost:11434/api/tags` deve rispondere JSON. Con una RTX 4070
Super vanno bene anche `llama3.1:8b` o `qwen2.5:7b`; il modello si cambia da **Impostazioni**.

## Avvio

```bash
npm install
npm run tauri dev     # dev server + finestra desktop
npm run tauri build   # eseguibile + installer in src-tauri/target/release
```

Claude API (opzionale): imposta la chiave **prima** di avviare, poi attiva "Usa Claude API" nelle
impostazioni.

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."
npm run tauri dev
```

Il modello usato è `claude-opus-5`. Senza chiave l'app resta su Ollama.

---

## Come ragiona Logica

Il flusso **non** è affidato tutto al modello: le decisioni che toccano il disco sono riconosciute
da regole deterministiche in Rust (`ai_conversationalist::classify`), al modello resta il
linguaggio. Così l'app funziona anche con un 7B che sbaglia il JSON, e continua a rispondere pure
senza AI.

| Cosa scrivi | Intento | Cosa succede |
|---|---|---|
| "organizza Downloads", "sistema il Desktop", `"D:\foto"` | `Scan` | scansione + riepilogo e domanda sul criterio |
| "per tipo" / "per data" / "per progetto" / "1" "2" "3" | `Propose` | schema + anteprima nel blocco proposta |
| "applica", "vai", "perfetto" (frase corta) | `Apply` | sposta i file, registra l'undo |
| "annulla", "torna indietro", "rollback" | `Undo` | rimette tutto dov'era |
| tutto il resto | `Chat` | risposta del modello, con lo stato reale iniettato nel prompt |

In chat: **📁** aggiunge cartelle col selettore di sistema, **Ctrl+Z** annulla l'ultima
operazione, **Interrompi** ferma uno spostamento in corso (i file già mossi tornano indietro).
Le cartelle monitorate impostate nelle preferenze compaiono come scorciatoie in ogni chat nuova.

Criteri di organizzazione:

- **per tipo** — lo schema lo propone l'AI (con fallback deterministico per estensione se la
  risposta non è valida); le regole sono pattern sul nome (`*.pdf`, `img_*`, `*fattura*`).
- **per data** — `2024/03 - Marzo` calcolato dalla data di modifica.
- **per anno** — `2024`.
- **per progetto** — primo livello del percorso di provenienza.

I criteri sui metadati non passano dalle regole sul nome: la destinazione si ricava dal file, cosa
che un pattern non può esprimere.

## Sicurezza

- **Nessun file viene mai cancellato**: solo `rename` (o copia + rimozione dell'originale tra
  volumi diversi). `remove_dir` tocca solo cartelle vuote create dall'app.
- Destinazioni sanificate: niente `..`, niente percorsi assoluti, niente caratteri illegali su
  Windows, massimo 4 livelli.
- Pre-flight: ogni destinazione deve restare dentro la cartella scelta, ogni sorgente deve esistere.
- Conflitti di nome risolti con suffisso `_1`, `_2`, … **prima** di toccare il disco.
- Applicazione su più cartelle = operazione unica: se una fallisce si annullano anche le altre.
- "tutto il PC" resta limitato a Download, Desktop e Documenti.
- Gli id delle chat accettano solo `[A-Za-z0-9_-]`: nessuna scrittura fuori da `~/.logica/chats/`.

---

## Storage

```
~/.logica/
├── config.json              cartelle monitorate, profilo, preferenze testuali
├── preferences.json         tema, motore AI, URL e modello Ollama
├── operations_log.json      ultime 50 operazioni, con i log per l'undo
└── chats/
    └── chat_20260829_142530_a1b2.json
```

Una chat salvata contiene metadati, messaggi e ultima proposta:

```json
{
  "id": "20260829_142530_a1b2",
  "title": "organizza Downloads",
  "created": "2026-08-29T14:25:30Z",
  "updated": "2026-08-29T14:31:02Z",
  "folders": ["C:\\Users\\me\\Downloads"],
  "files_moved": 234,
  "folders_created": 9,
  "applied_on": "2026-08-29T14:31:02Z",
  "messages": [
    { "id": "m1", "role": "ai", "content": "Ciao, sono Logica…", "timestamp": "…", "type": "text" }
  ],
  "final_proposal": { "schema": {}, "rules": [], "rationale": "…", "source": "ollama", "grouping": "extension" }
}
```

La scansione **non** viene salvata (può essere di 100k file): riaprendo una chat vecchia le
cartelle vengono riesaminate al primo messaggio.

L'undo sopravvive al riavvio dell'app perché i log stanno in `operations_log.json`; in più ogni
cartella organizzata conserva `.file-organizer-undo.json` come traccia.

---

## Struttura

```
file-organizer-ai/
├── src/
│   ├── components/
│   │   ├── ChatInterface.tsx              header, messaggi, typing, composer, undo
│   │   ├── OrganizationProposalBlock.tsx  albero + tabella + statistiche + azioni
│   │   ├── Sidebar.tsx                    chat, statistiche, impostazioni
│   │   ├── SettingsModal.tsx              cartelle, profilo, motore AI, tema
│   │   └── NotificationToast.tsx          toast
│   ├── pages/ChatPage.tsx                 stato della conversazione, invoke, persistenza
│   ├── types.ts                           interfacce condivise col backend
│   ├── App.tsx / main.tsx / App.css       shell e tema (scuro + chiaro)
└── src-tauri/src/
    ├── main.rs                  setup Tauri, stato in memoria, comandi
    ├── ai_conversationalist.rs  intenti, prompt di Logica, chiamate chat, testi
    ├── ai_analyzer.rs           prompt schema, Ollama/Claude, validazione, fallback
    ├── scanner.rs               scansione file system
    ├── file_mapper.rs           regole/criteri → piano + conflitti + anteprima
    ├── file_operations.rs       mkdir/move/rollback/undo, piani multipli
    └── storage.rs               ~/.logica: chat, preferenze, log operazioni
```

### Comandi Tauri

| Comando | Argomenti (JS) | Ritorna |
|---|---|---|
| `chat_message` | `message`, `chatId`, `history`, `config`, `proposal` | `ChatReply` |
| `scan_folders` | `chatId`, `folders` | `ChatReply` |
| `preview_proposal` | `chatId`, `proposal` | `PlanPreview` |
| `apply_proposal` | `chatId`, `proposal` | `ApplyResult` |
| `undo_last_action` | `chatId` | `string` |
| `has_undo` | `chatId` | `boolean` |
| `cancel_apply` | — | — |
| `save_chat` / `load_chat` / `list_chats` / `delete_chat` | `chat` / `chatId` | `—` / `StoredChat` / `ChatMetadata[]` / `—` |
| `get_preferences` / `set_preferences` | — / `preferences` | `Preferences` / `—` |
| `get_config` / `set_config` | — / `config` | `Config` / `—` |
| `check_ai_status` | `ollamaUrl` | `{ ollama, claude, models }` |
| `open_folder_in_explorer` | `path` | — |

Eventi: `scan-progress` (`{scanned, current}`), `apply-progress` (`{done, total, current}`).

---

## Esempi di conversazione

**Semplice**

```
User:   organizza Downloads
Logica: Ho scansionato Downloads: 234 file trovati per 12.480 MB.
        Tipi più frequenti: .pdf (45), .mp4 (78), .jpg (33), .zip (12), .docx (9).
        Come preferisci organizzarli?
        1) Per tipo  2) Per data  3) Per progetto
User:   per tipo
Logica: Ecco lo schema per tipo di file (schema suggerito dal modello locale)…
        [SCHEMA PROPOSTO: albero + tabella + 234 file / 9 cartelle / 3 conflitti]
User:   applica
Logica: ✅ Fatto! Ho spostato 234 file in 9 cartelle.
        Se non ti convince, scrivi "annulla" e rimetto tutto com'era.
```

**Più cartelle e ripensamento**

```
User:   sistema Downloads e Desktop
Logica: Ho scansionato Downloads, Desktop: 512 file trovati per 30.112 MB…
User:   per data
Logica: Ecco lo schema per anno e mese. Riguarda 512 file…
        [SCHEMA PROPOSTO: 2025/11 - Novembre, 2026/01 - Gennaio, …]
User:   vai
Logica: ✅ Fatto! Ho spostato 512 file in 14 cartelle.
User:   annulla
Logica: Fatto: ho rimesso 512 file dove stavano prima.
```

Le due cartelle sono trattate come un'unica operazione: l'undo le riporta indietro insieme.

---

## Troubleshooting

**"offline · nessun modello AI"** — Ollama non risponde su `localhost:11434`. Avvia `ollama serve`
e controlla `ollama list`. L'app resta usabile: i criteri per data/progetto sono deterministici e
"per tipo" ha lo schema di riserva per estensione.

**La proposta arriva sempre dal "fallback"** — il modello non restituisce JSON valido. Prova
`ollama pull llama3.1:8b` e cambialo nelle impostazioni; la console dev stampa il motivo
(`[ai_analyzer] fallback attivato: …`).

**Logica non capisce la cartella** — usa il pulsante 📁 accanto al messaggio, oppure scrivi il
percorso tra virgolette: `organizza "D:\Archivio 2024"`.

**Scansione lentissima** — hai puntato a una cartella enorme. Le cartelle di sistema,
`node_modules`, `.git` e simili sono comunque saltate.

**"Percorso lungo N caratteri"** — la destinazione supera i 260 caratteri di Windows: rinomina
qualche cartella con "Modifica" nel blocco proposta.

**`cargo` non trovato / errore `link.exe`** — manca Rust o il workload C++ dei Build Tools.

---

## Sviluppo

```bash
cd src-tauri && cargo test        # 26 test: intenti, schema, date, storage, ciclo completo, multi-cartella
cargo test -- --ignored           # 2 test live contro Ollama: turno di chat + proposta vera
                                  # (richiedono `ollama serve` e il modello scaricato;
                                  #  FOAI_TEST_MODEL cambia il modello usato)
npm run build                     # typecheck TypeScript + bundle Vite
```
