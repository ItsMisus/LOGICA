# Logica — assistente di conversazione desktop

App desktop (Tauri 2 + React + Rust) con un assistente con cui parlare: domande, ragionamenti,
codice, testi. La risposta arriva **in streaming**, token per token, e le conversazioni restano
sul disco in `~/.logica/`.

Il motore è **Ollama in locale** (default, offline e gratis) oppure la **Claude API** (opzionale).
Quando una domanda ha bisogno di dati aggiornati, Logica **cerca su internet** da sola e risponde
citando le pagine che ha letto. Se le autorizzi una cartella, **guarda anche dentro i tuoi file**:
li elenca, li legge e ci cerca dentro, in sola lettura.

```
┌──────────────────┬─────────────────────────────────────────┐
│ [+ Nuova chat]   │  ☰  Logica                              │
│ [cerca…]         │                                         │
│                  │              ✳ Buonasera                │
│ OGGI             │      ┌───────────────────────────────┐  │
│ • Regex in Rust  │      │ Scrivi a Logica…         [↑]  │  │
│ • Lettera al…    │      └───────────────────────────────┘  │
│                  │   [Spiegami una cosa] [Rivedi codice]   │
│ Impostazioni     │                                         │
└──────────────────┴─────────────────────────────────────────┘
```

Impianto grafico squadrato: fondo quasi nero, colonna di lettura stretta e centrata, bolla solo per
i messaggi dell'utente, bordeaux come unico accento, micro-etichette maiuscole spaziate. Tema scuro
di default, chiaro nelle impostazioni.

---

## Requisiti

| Cosa | Versione | Note |
|---|---|---|
| Node.js | ≥ 18 | |
| Rust | stable (rustup) | **necessario per compilare il backend** |
| Ollama | qualsiasi | serve per rispondere in locale |
| Build tools Windows | MSVC + WebView2 | Visual Studio Build Tools, workload C++ |

```powershell
winget install --id Rustlang.Rustup -e
rustup default stable

winget install --id Ollama.Ollama -e
ollama pull llama3.1:8b    # modello di default
```

Verifica Ollama: `curl http://localhost:11434/api/tags` deve rispondere JSON. Il modello si cambia
da **Impostazioni**, che elenca quelli installati. `llama3.1:8b` è il default; `mistral` (7B) è il
minimo sindacale, `qwen2.5:14b` ragiona meglio se la GPU regge.

## Avvio

```bash
npm install
npm run tauri dev     # dev server + finestra desktop
npm run tauri build   # eseguibile + installer in src-tauri/target/release
```

Claude API (opzionale): imposta la chiave **prima** di avviare, poi scegli "Claude API" nelle
impostazioni.

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."
npm run tauri dev
```

Il modello usato è `claude-opus-5`. Senza chiave l'app resta su Ollama.

---

## Com'è fatta la conversazione

- **Streaming**: il backend legge la risposta riga per riga (NDJSON per Ollama, SSE per Claude) e
  la rilancia al frontend come eventi `assistant-delta`; il testo compare mentre viene generato.
- **Interrompi**: il pulsante quadrato ferma la generazione e **tiene** il testo già arrivato.
- **Rigenera**: rimanda l'ultima domanda senza l'ultima risposta.
- **Modifica**: qualsiasi messaggio dell'utente si riscrive e riparte da lì. Quello che veniva dopo
  viene lasciato indietro: per tenere anche la strada vecchia si duplica prima la conversazione
  (menu `⋯` nella barra laterale).
- **Storico**: al modello vanno gli ultimi 30 turni della chat corrente, più le istruzioni
  personalizzate scritte nelle impostazioni, la data di oggi e quello che ha imparato nelle altre
  conversazioni (vedi *Memoria*).
- **Titolo**: dopo il primo scambio è il modello stesso a proporlo; se non ne esce niente di
  sensato resta la prima frase dell'utente.
- **Markdown**: titoli, elenchi, citazioni, tabelle, codice con pulsante *Copia*. Il parser è in
  `src/components/Markdown.tsx`, senza librerie e senza `dangerouslySetInnerHTML`: il testo del
  modello non può iniettare HTML.

Il carattere dell'assistente sta tutto in `SYSTEM_PROMPT` (`src-tauri/src/assistant.rs`): risposta
prima delle premesse, niente adulazione, una domanda sola quando la richiesta è ambigua, e il
divieto di fingere accessi che non ha. Quel pezzo sui file ha due versioni, scelte a seconda che gli
strumenti siano accesi o spenti: dire insieme "non puoi leggere niente" e "ecco le cartelle che puoi
leggere" manda in confusione un modello piccolo.

Scorciatoie: **Invio** manda, **Shift+Invio** va a capo, **Ctrl+N** apre una conversazione nuova.

### Allegati

Il pulsante `+` del composer apre il dialogo di sistema; il contenuto lo legge il backend
(`src-tauri/src/attachments.rs`), l'app non copia niente da nessuna parte.

| Tipo | Cosa arriva al modello | Limite |
|---|---|---|
| Testo (`.txt`, `.md`, `.csv`, sorgenti, …) | il testo, tagliato a 12.000 caratteri | 2 MB |
| PDF | il testo estratto | 20 MB |
| DOCX | il testo dei paragrafi | 20 MB |
| Immagini (`png`, `jpeg`, `gif`, `webp`) | il file in base64 | 3 MB |

Dettagli che si notano solo quando mancano: un file non UTF-8 viene riprovato in **Windows-1252**
prima di dichiararlo illeggibile; un PDF fatto di scansioni lo dice invece di tornare vuoto; un PDF
malfatto che manda in panico l'estrattore diventa un messaggio, non un crash dell'app.

Le immagini le guarda solo un modello che vede: **Claude**, oppure un modello Ollama con la vista
(`llava`, `llama3.2-vision`, …). Con un modello solo testuale l'app lo dice prima di mandare. Valgono
per il turno in cui sono allegate: nello storico resta l'etichetta, non i byte.

### Conversazioni

- **Ricerca** (`search_chats`): cerca dentro i messaggi, non solo nei titoli, e mostra il pezzo di
  testo in cui la parola compare. Confronto in minuscolo, sottostringa secca.
- **Menu `⋯`** su ogni conversazione: **rinomina** (non la fa risalire in cima: la data resta quella
  dell'ultimo messaggio), **duplica** (per cambiare strada senza perdere quella di prima),
  **esporta** in Markdown o JSON, **elimina** con conferma.
- L'esportazione sul desktop passa dal dialogo di sistema; da browser il file si scarica.

### Ricerca su internet

Prima di rispondere, l'app fa un giro in più (`src-tauri/src/web.rs`):

1. **Link scritti dall'utente**: se nel messaggio c'è un indirizzo `http(s)://`, l'app apre fino a
   due pagine e ne passa il testo al modello. Succede anche a ricerca spenta: l'ha chiesto l'utente.
2. **Serve cercare?** Se non ci sono link, il modello riceve una domanda di servizio separata
   («questa richiesta ha bisogno di internet? se sì, scrivi la query») e risponde con una riga o
   con `NO`. Decidere e rispondere in un colpo solo è il tipo di doppio compito su cui un 7B
   sbaglia; separarli costa una chiamata corta e funziona anche in locale.
3. **Ricerca**: la query va a **DuckDuckGo** (endpoint HTML, nessuna chiave API) e, se DuckDuckGo
   blocca l'indirizzo perché ha ricevuto troppe richieste, a **Bing**. Delle prime pagine si legge
   il testo — via tag, script e CSS — tagliato a 6.000 caratteri l'una.
4. **Risposta**: i risultati entrano nel prompt di sistema di quel solo turno, con l'ordine di
   fidarsi di quei dati e citare le fonti come link. Le pagine consultate compaiono come chip
   sotto la risposta e restano nella chat salvata.

Mentre cerca, la chat mostra cosa sta facendo (evento `assistant-status`): *Cerco su internet: …*,
*Leggo …*. Se la ricerca non trova niente o la rete è giù, il modello lo dice invece di inventare.

L'interruttore sta in **Impostazioni → Internet** ed è acceso di default; da browser non si cambia,
come il resto della configurazione del motore. Fuori esce **la query**, non i tuoi file (quelli
escono solo se accendi gli strumenti sui file *e* usi la Claude API). Gli
indirizzi locali e di rete interna (`localhost`, `192.168.*`, `*.local`, `169.254.169.254`) sono
rifiutati, anche dopo un redirect: il modello non può usare l'app per bussare ai servizi di casa.

### Memoria

Fino a qui ogni chat nuova ripartiva da zero: l'assistente si ripresentava e ti chiedeva di nuovo
quello che gli avevi già detto. Ora, **dopo** che la risposta è a schermo, parte un secondo compito
che legge lo scambio e si annota solo quello che varrà la pena sapere anche fra un mese: come ti
chiami, cosa usi, a cosa stai lavorando, come vuoi le risposte. Gira in background: l'attesa della
risposta non cambia di un secondo.

I fatti stanno in `~/.logica/memory.json`, uno per riga, e finiscono nel prompt di sistema di tutte
le chat successive. Con i freni:

- **3 fatti** al massimo da uno scambio, **60** in tutto; oltre, i più vecchi escono. Una memoria
  che cresce senza limite si mangia la finestra di contesto che serve alla domanda.
- Una riga più lunga di 200 caratteri non è un fatto, è un riassunto: viene scartata.
- Un fatto che c'è già non entra due volte. «L'utente usa Rust» e «L'utente usa Rust al lavoro» sono
  lo stesso fatto: senza questo controllo la memoria si riempirebbe di varianti della stessa riga.
- L'estrattore deve parlare *dell'utente*: una riga che riassume la risposta («la regex in Rust si
  scrive…») viene buttata prima di arrivare al file.

In **Impostazioni → Memoria** c'è l'interruttore (acceso di default), l'elenco completo di quello
che sa e un pulsante *Dimentica* per riga. Spento, smette sia di annotare sia di leggere; quello che
ha già imparato resta finché non lo cancelli.

Nel prompt la memoria e «ricordi solo questa chat» sono alternative, mai insieme: due istruzioni
opposte nello stesso prompt fanno scegliere a caso un modello piccolo.

### Accesso ai file

Di default l'assistente non vede niente del disco e lo dice. In **Impostazioni → File** si
autorizzano una o più cartelle (dal dialogo di sistema, non scrivendo un percorso) e si accende
l'interruttore: da quel momento il modello ha quattro strumenti, tutti in **sola lettura**.

| Strumento | Cosa fa | Freni |
|---|---|---|
| `list_dir` | elenca una cartella: nomi, dimensioni, data | 200 voci |
| `read_file` | legge un file: testo, sorgenti, PDF, DOCX | 12.000 caratteri, stessa lettura degli allegati |
| `find_files` | cerca per pezzo di nome, nelle sottocartelle | 40 risultati, 6 livelli, 4.000 file visitati |
| `search_in_files` | cerca un testo dentro i file, riga per riga | 40 righe, file oltre 2 MB e binari saltati |

Il recinto è lo stesso dell'accesso da browser (`server/guard.rs`): un percorso fuori dalle cartelle
autorizzate viene rifiutato prima di toccare il disco, `..` compreso, e un link simbolico che punta
fuori non funziona come scorciatoia. `node_modules`, `.git`, `target` e simili non vengono nemmeno
percorsi: costano minuti e non contengono mai la risposta.

Il modello può fare fino a 6 giri di strumenti in un turno; all'ultimo gli strumenti gli vengono
tolti, così conclude con quello che ha raccolto invece di chiedere ancora. Ogni esito torna al
modello tagliato a 8.000 caratteri. Mentre guarda, la chat scrive cosa sta facendo (*Guardo in …*,
*Leggo …*) e sotto la risposta restano i chip di quello che ha aperto, salvati nella chat.

**Non può scrivere, spostare, rinominare o cancellare niente.** Una scrittura decisa da un modello
locale senza una conferma davanti agli occhi non vale la comodità.

Due cose da sapere prima di accendere l'interruttore: quello che il modello legge finisce nel
prompt, quindi con la **Claude API** esce in rete (con Ollama no); e l'interruttore, come
l'indirizzo del motore, **non si tocca dal browser** — una sessione remota non può accendersi da
sola la lettura dei file.

---

## Storage

```
~/.logica/
├── config.json              istruzioni personalizzate, cartelle raggiungibili dal browser
├── preferences.json         tema, motore AI, URL e modello Ollama, ricerca su internet, accesso ai file, memoria
├── memory.json              i fatti imparati sull'utente, uno per riga
└── chats/
    └── chat_20260829_142530_a1b2.json
```

Una chat salvata:

```json
{
  "id": "20260829_142530_a1b2",
  "title": "Regex in Rust",
  "created": "2026-08-29T14:25:30Z",
  "updated": "2026-08-29T14:31:02Z",
  "messages": [
    { "id": "m1", "role": "user", "content": "come si scrive…", "timestamp": "…", "type": "text" },
    { "id": "m2", "role": "ai", "content": "…", "timestamp": "…", "type": "text",
      "sources": [{ "title": "…", "url": "https://…", "snippet": "…" }] }
  ]
}
```

Gli id delle chat accettano solo `[A-Za-z0-9_-]`: nessuna scrittura fuori da `~/.logica/chats/`.

Gli allegati restano dentro il file della chat: il testo estratto, e per le immagini il base64. Una
conversazione con immagini pesa quindi quanto le immagini (al più 3 MB l'una, un terzo in più una
volta codificate). In cambio la chat è completa da sola: si può riaprire, modificare un messaggio e
rigenerare anche se il file originale nel frattempo è stato spostato.

---

## Accesso da browser

Logica può rispondere anche a un browser su un altro dispositivo della rete locale. Il server vive
dentro il processo dell'app: un solo proprietario di `~/.logica`, nessuna scrittura concorrente
sulle chat. Si accende da Impostazioni → *Accesso da browser*; da spento non apre nessuna porta.

Prima di poterlo accendere serve una password (minimo 12 caratteri), salvata solo come hash
Argon2id in `~/.logica/server.json`.

### Cosa protegge cosa

| Strato | File | Cosa garantisce |
|---|---|---|
| TLS | `server/tls.rs` | Certificato auto-firmato: password e cookie non viaggiano in chiaro |
| Sessione | `server/auth.rs` | Password Argon2id, cookie `HttpOnly`/`Secure`/`SameSite=Strict`, scadenza per inattività (30 min) e assoluta (12 h), sessione legata all'indirizzo IP, blocco di 15 minuti dopo 5 tentativi falliti |
| CSRF | `server/api.rs` | Token per sessione nell'header `X-Logica-Csrf`, più confronto fra `Origin` e `Host` su ogni scrittura |
| Recinto | `server/guard.rs` | Ogni percorso deve stare sotto `monitored_folders`: rifiuta `..`, gli stream alternativi NTFS (`file:stream`), i nomi di dispositivo (`NUL`, `COM1`) e i link che escono |
| Intestazioni | `server/api.rs` | CSP senza sorgenti esterne, `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, corpo limitato a 8 MB |

### Cosa il browser non può fare, nemmeno autenticato

- **Allargare il recinto**: `set_config` rifiuta ogni modifica a `monitored_folders`.
- **Spostare il motore AI**: `set_preferences` rifiuta ogni modifica a `ollama_url`, e `ollama_url`
  nel corpo delle richieste viene comunque ignorato a favore di quello salvato sul PC. Senza questo,
  una sessione valida basterebbe a far spedire altrove conversazioni e nomi dei file.
- **Allegare file**: gli allegati si leggono per percorso dal dialogo di sistema, che nel browser
  non esiste; il pulsante non compare.

### Prima connessione

Il browser avvisa che il certificato non è verificato: è atteso, se lo firma Logica stessa. Prima
di accettare, confronta l'impronta SHA-256 mostrata dal browser con quella scritta nel pannello
delle impostazioni. Se non coincidono, non stai parlando con il tuo PC.

> Non aprire questa porta sul router. Non c'è nessuna protezione pensata per internet: per usarlo
> fuori casa serve una VPN (Tailscale o simili).

---

## Struttura

```
file-organizer-ai/
├── src/
│   ├── components/
│   │   ├── ChatView.tsx           header, thread, schermata iniziale, composer
│   │   ├── Message.tsx            bolla utente / risposta + azioni (copia, rigenera, modifica)
│   │   ├── Markdown.tsx           markdown minimo, senza dipendenze
│   │   ├── Composer.tsx           campo che cresce, invio/interrompi
│   │   ├── Sidebar.tsx            conversazioni per data, ricerca nel contenuto, menu azioni
│   │   ├── SettingsModal.tsx      motore AI, modello, internet, istruzioni, tema
│   │   └── NotificationToast.tsx  toast
│   │   ├── LoginScreen.tsx        accesso da browser (invisibile sul desktop)
│   │   └── RemoteAccessPanel.tsx  password, porta, impronta, cartelle raggiungibili
│   ├── pages/ChatPage.tsx         stato della chat, streaming, persistenza
│   ├── api.ts                     un solo modo di chiamare: IPC Tauri o HTTPS
│   ├── types.ts                   interfacce condivise col backend
│   └── App.tsx / main.tsx / App.css
└── src-tauri/src/
    ├── main.rs                  setup Tauri, stato in memoria, comandi
    ├── server/
    │   ├── mod.rs               accensione, spegnimento, stato del server
    │   ├── api.rs               rotte, sessione, CSRF, intestazioni
    │   ├── auth.rs              password Argon2id, sessioni, blocco tentativi
    │   ├── guard.rs             recinto dei percorsi
    │   └── tls.rs               certificato auto-firmato e impronta
    ├── assistant.rs             prompt, streaming Ollama/Claude, ricerca web, titolo
    ├── web.rs                   ricerca (DuckDuckGo, Bing), pagine, HTML → testo
    ├── attachments.rs           allegati: testo, PDF, DOCX, immagini in base64
    ├── tools.rs                 strumenti sui file: elenca, legge, cerca (sola lettura)
    ├── memory.rs                i fatti che l'assistente si porta fra una chat e l'altra
    ├── ai.rs                    configurazione del motore AI e stato della connessione
    └── storage.rs               ~/.logica: chat, ricerca, esportazione, preferenze
```

### Comandi Tauri usati dalla chat

| Comando | Argomenti (JS) | Ritorna |
|---|---|---|
| `talk` | `chatId`, `message`, `history`, `images`, `config` | `{ text, stopped, error, sources, tools }` |
| `stop_talk` | — | — |
| `suggest_title` | `question`, `answer`, `config` | `string \| null` |
| `read_attachments` | `paths` | `Attachment[]` |
| `save_chat` / `load_chat` / `list_chats` / `delete_chat` | `chat` / `chatId` | `—` / `StoredChat` / `ChatMetadata[]` / `—` |
| `search_chats` | `query` | `ChatMatch[]` |
| `rename_chat` | `chatId`, `title` | `ChatMetadata` |
| `duplicate_chat` | `chatId` | `ChatMetadata` |
| `export_chat` | `chatId`, `format` (`markdown` \| `json`) | `string` |
| `save_export` | `path`, `content` | `—` (solo desktop: non ha una rotta sul server) |
| `get_preferences` / `set_preferences` | — / `preferences` | `Preferences` / `—` |
| `get_config` / `set_config` | — / `config` | `Config` / `—` |
| `get_memory` / `forget_fact` / `clear_memory` | — / `factId` / — | `Fact[]` / `—` / `—` |
| `check_ai_status` | `ollamaUrl` | `{ ollama, claude, models }` |

Eventi: `assistant-delta` (`{ chat_id, text }`), un pezzo di risposta alla volta, e
`assistant-status` (`{ chat_id, text }`), cosa sta facendo l'app prima di scrivere.

## Test

```bash
cd src-tauri
cargo test                                   # 78 test offline
cargo test -- --ignored live_ollama_stream   # streaming vero contro Ollama
cargo test -- --ignored live_search          # ricerca vera (DuckDuckGo/Bing)
cargo test -- --ignored live_research        # ricerca + lettura delle pagine
cargo test -- --ignored live_web_flow        # giro completo: decide, cerca, risponde
```

I test live con Ollama richiedono `ollama serve` e il modello scaricato; `FOAI_TEST_MODEL` cambia
il modello usato dai test. Quelli di ricerca richiedono internet: `live_search` fallisce se
entrambi i motori rispondono con una pagina di blocco, e in quel caso il parsing va riguardato.
