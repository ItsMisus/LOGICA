//! Le cento richieste su cui si misura l'assistente.
//!
//! Ogni caso porta con se' come si giudica, perche' un caso senza criterio non
//! e' una misura ma un'impressione. Le attese sono scritte in modo da poter
//! essere sbagliate: se un caso non puo' fallire non dice niente.
//!
//! Due scelte che contano piu' delle altre.
//!
//! I dati da trovare sono codici e importi che **non si possono indovinare**
//! (`FD9812-004473`, `1.240,00`). Un modello che li scrive li ha letti; un
//! modello che non li scrive non ha letto. Senza questo, una risposta
//! plausibile e una risposta vera si assomigliano troppo.
//!
//! Diversi casi chiedono qualcosa che nei documenti **non c'e'**: il colore
//! delle finestre, un file che non esiste, il prezzo del volo. Li' la risposta
//! giusta e' ammettere di non saperlo, e `forbid` elenca cosa vorrebbe dire
//! essersela inventata. Sono i casi piu' importanti dell'insieme: misurare
//! solo domande con risposta premia il modello che risponde sempre.

/// A quale parte del programma appartiene il caso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Category {
    Chat,
    Memoria,
    Rag,
    File,
    Web,
    Allegati,
    Strumenti,
    Ambigue,
}

impl Category {
    pub const ALL: &'static [Category] = &[
        Category::Chat,
        Category::Memoria,
        Category::Rag,
        Category::File,
        Category::Web,
        Category::Allegati,
        Category::Strumenti,
        Category::Ambigue,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Chat => "chat normale",
            Category::Memoria => "memoria",
            Category::Rag => "documenti",
            Category::File => "strumenti sui file",
            Category::Web => "ricerca web",
            Category::Allegati => "allegati",
            Category::Strumenti => "uso degli strumenti",
            Category::Ambigue => "richieste ambigue",
        }
    }

    /// Il nome corto con cui si filtra da riga di comando.
    pub fn key(self) -> &'static str {
        match self {
            Category::Chat => "chat",
            Category::Memoria => "memoria",
            Category::Rag => "documenti",
            Category::File => "file",
            Category::Web => "web",
            Category::Allegati => "allegati",
            Category::Strumenti => "strumenti",
            Category::Ambigue => "ambigue",
        }
    }
}

/// Cosa ci si aspetta dagli strumenti sui file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tools {
    /// Non deve toccarne nessuno: la risposta e' gia' davanti a lui.
    Nessuno,
    /// Deve usarne almeno uno, quale non importa.
    Qualcuno,
    /// Deve usarne almeno questo numero: i casi in catena.
    Almeno(usize),
    /// Devono comparire tutti questi nomi.
    Questi(&'static [&'static str]),
    /// Il caso non parla di strumenti.
    Ignora,
}

/// Cosa ci si aspetta dalla ricerca su internet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Web {
    /// Deve cercare: la risposta non ce l'ha in pancia.
    Cerca,
    /// Non deve cercare: la sa gia', o non c'e' niente da cercare.
    No,
    Ignora,
}

#[derive(Debug, Clone, Copy)]
pub struct Case {
    pub id: &'static str,
    pub category: Category,
    /// Il messaggio dell'utente.
    pub prompt: &'static str,
    /// Solo per la memoria: la risposta dell'assistente nello scambio da cui
    /// si deve imparare.
    pub answer: &'static str,
    /// Ogni gruppo va soddisfatto, e per soddisfarlo basta una delle
    /// alternative. Cosi' "quattro" e "4" contano uguale senza allargare la
    /// maglia su quello che conta.
    pub expect: &'static [&'static [&'static str]],
    /// Se compare anche una sola di queste, la risposta e' inventata.
    pub forbid: &'static [&'static str],
    pub tools: Tools,
    /// Il file che la ricerca nei documenti deve pescare, quando il caso la
    /// riguarda.
    pub retrieval: Option<&'static str>,
    /// Il testo dell'allegato va incollato sotto al messaggio, come fa la
    /// finestra della chat.
    pub attachment: bool,
    pub web: Web,
}

/// I valori comuni, per non ripetere otto campi in cento righe.
const BASE: Case = Case {
    id: "",
    category: Category::Chat,
    prompt: "",
    answer: "",
    expect: &[],
    forbid: &[],
    tools: Tools::Ignora,
    retrieval: None,
    attachment: false,
    web: Web::Ignora,
};

pub const CASES: &[Case] = &[
    /* ---------------------------- chat normale ---------------------------- */
    // Con gli strumenti accesi. Il punto non e' se sa la risposta — sono cose
    // che sa — ma se resiste alla tentazione di aprire file e di cercare su
    // internet per domande che non lo richiedono.
    Case {
        id: "chat-01",
        prompt: "Qual e' la capitale dell'Australia?",
        expect: &[&["Canberra"]],
        // La capitale sbagliata che dicono tutti: se compare, ha risposto a
        // memoria senza pensarci.
        forbid: &["Sydney"],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-02",
        prompt: "Quanto fa 17 per 23?",
        expect: &[&["391"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-03",
        prompt: "Chi ha scritto Il nome della rosa?",
        expect: &[&["Eco"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-04",
        prompt: "In che anno e' caduto il muro di Berlino?",
        expect: &[&["1989"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-05",
        prompt: "Qual e' il pianeta piu' grande del sistema solare?",
        expect: &[&["Giove"]],
        forbid: &["Saturno"],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-06",
        prompt: "Quanto fa il 15 per cento di 240?",
        expect: &[&["36"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-07",
        prompt: "Qual e' la formula chimica dell'acqua?",
        expect: &[&["H2O", "H₂O"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-08",
        prompt: "Quanti giorni ha febbraio in un anno bisestile?",
        expect: &[&["29"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-09",
        prompt: "Che lingua si parla in Brasile?",
        expect: &[&["portoghese"]],
        forbid: &["spagnolo"],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-10",
        prompt: "Spiegami in due righe cos'e' la ricorsione in programmazione.",
        expect: &[&["se stess", "se' stess", "sé stess", "richiama", "chiama se"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-11",
        prompt: "Qual e' la differenza fra un kW e un kWh?",
        expect: &[&["potenza"], &["energia", "consum"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-12",
        prompt: "Cosa significa la sigla API?",
        expect: &[&["interfaccia", "Application"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-13",
        prompt: "Traduci in spagnolo: buongiorno.",
        expect: &[&["Buenos d", "buenos d", "Buen d", "buen d"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-14",
        prompt: "Qual e' il contrario di effimero?",
        expect: &[&["duraturo", "duratura", "perenne", "permanente", "eterno", "durevole"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-15",
        prompt: "Quante zampe ha un ragno?",
        expect: &[&["otto", "8"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-16",
        prompt: "A cosa serve un indice in un database?",
        expect: &[&["veloc", "rapid", "ricerc"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-17",
        prompt: "Cos'e' la fotosintesi clorofilliana, in breve?",
        expect: &[&["luce", "solare"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-18",
        prompt: "Quanto fa 144 diviso 12?",
        expect: &[&["12"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-19",
        prompt: "Cos'e' un puntatore nel linguaggio C?",
        expect: &[&["indirizzo"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "chat-20",
        prompt: "Elenca i primi cinque numeri primi.",
        expect: &[&["2"], &["3"], &["5"], &["7"], &["11"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    /* ------------------------------- memoria ------------------------------ */
    // Qui non si misura una risposta ma cosa resta dello scambio. La meta' dei
    // casi non deve lasciare niente: una memoria che impara tutto e' rumore che
    // si porta dietro per sempre.
    Case {
        id: "mem-01",
        category: Category::Memoria,
        prompt: "Mi chiamo Franco e faccio l'elettricista.",
        answer: "Piacere Franco.",
        expect: &[&["Franco"], &["elettricista"]],
        ..BASE
    },
    Case {
        id: "mem-02",
        category: Category::Memoria,
        prompt: "Quanto fa 8 per 9?",
        answer: "72.",
        // Niente: un conto fatto oggi non serve fra un mese.
        expect: &[],
        forbid: &["72"],
        ..BASE
    },
    Case {
        id: "mem-03",
        category: Category::Memoria,
        prompt: "Rispondimi sempre in modo conciso, senza preamboli.",
        answer: "Va bene.",
        expect: &[&["concis", "brev", "preamboli", "sintetic"]],
        ..BASE
    },
    Case {
        id: "mem-04",
        category: Category::Memoria,
        prompt: "Sto ristrutturando una casa in Trentino, i lavori finiscono a ottobre.",
        answer: "Un lavoro lungo.",
        expect: &[&["ristruttur", "casa", "Trentino"]],
        ..BASE
    },
    Case {
        id: "mem-05",
        category: Category::Memoria,
        prompt: "Che ore sono adesso a Tokyo?",
        answer: "Sono circa le tre del pomeriggio.",
        expect: &[],
        forbid: &["Tokyo"],
        ..BASE
    },
    Case {
        id: "mem-06",
        category: Category::Memoria,
        prompt: "Uso Windows 11 e programmo in Rust.",
        answer: "Buona scelta.",
        expect: &[&["Rust"]],
        ..BASE
    },
    Case {
        id: "mem-07",
        category: Category::Memoria,
        prompt: "Sono allergico al lattosio, tienine conto quando mi consigli da mangiare.",
        answer: "Me lo segno.",
        expect: &[&["lattosio", "allergic"]],
        ..BASE
    },
    Case {
        id: "mem-08",
        category: Category::Memoria,
        prompt: "Spiegami la differenza fra TCP e UDP.",
        answer: "TCP garantisce la consegna, UDP no.",
        expect: &[],
        forbid: &["TCP", "UDP"],
        ..BASE
    },
    Case {
        id: "mem-09",
        category: Category::Memoria,
        prompt: "Ho due gatti, si chiamano Pepe e Nina.",
        answer: "Bei nomi.",
        expect: &[&["gatt"]],
        ..BASE
    },
    Case {
        id: "mem-10",
        category: Category::Memoria,
        prompt: "Qual e' la radice quadrata di 144?",
        answer: "12.",
        expect: &[],
        forbid: &["144", "radice"],
        ..BASE
    },
    Case {
        id: "mem-11",
        category: Category::Memoria,
        prompt: "Preferisco le risposte con esempi di codice commentati.",
        answer: "D'accordo.",
        expect: &[&["codice", "esempi"]],
        ..BASE
    },
    Case {
        id: "mem-12",
        category: Category::Memoria,
        prompt: "Mia moglie Elena insegna alle medie.",
        answer: "Capito.",
        expect: &[&["Elena", "moglie"]],
        ..BASE
    },
    Case {
        id: "mem-13",
        category: Category::Memoria,
        prompt: "Traduci 'grazie' in tedesco.",
        answer: "Danke.",
        expect: &[],
        forbid: &["tedesc", "Danke"],
        ..BASE
    },
    Case {
        id: "mem-14",
        category: Category::Memoria,
        prompt: "Lavoro part time, solo il lunedi' e il mercoledi'.",
        answer: "Segnato.",
        expect: &[&["part time", "luned", "mercoled"]],
        ..BASE
    },
    Case {
        id: "mem-15",
        category: Category::Memoria,
        prompt: "Riassumimi questo articolo sul cambiamento climatico.",
        answer: "L'articolo sostiene che le emissioni sono calate del 3 per cento.",
        expect: &[],
        forbid: &["clima", "emissioni"],
        ..BASE
    },
    /* ------------------------------ documenti ----------------------------- */
    // Domande scritte apposta con parole che nei documenti non compaiono: "il
    // padrone di casa" per il contratto d'affitto, "i grassi nel sangue" per le
    // analisi. Cercare per parole non basterebbe, e infatti gli strumenti sui
    // file qui restano spenti.
    Case {
        id: "rag-01",
        category: Category::Rag,
        prompt: "Quanto devo dare al padrone di casa ogni mese?",
        expect: &[&["650"]],
        retrieval: Some("contratto_affitto"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-02",
        category: Category::Rag,
        prompt: "Se il frigo si rompe, fino a quando me lo aggiustano gratis?",
        expect: &[&["24 mesi", "due anni", "2 anni", "ottobre 2026"]],
        retrieval: Some("garanzia_frigorifero"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-03",
        category: Category::Rag,
        prompt: "Quanto ho speso l'ultima volta per far controllare il riscaldamento?",
        expect: &[&["118"]],
        retrieval: Some("caldaia_manutenzione"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-04",
        category: Category::Rag,
        prompt: "Quanti soldi ho lasciato come garanzia quando ho preso l'appartamento?",
        expect: &[&["1.300", "1300"]],
        retrieval: Some("contratto_affitto"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-05",
        category: Category::Rag,
        prompt: "A che ora parte il volo per il Portogallo?",
        expect: &[&["7:15", "7.15"]],
        retrieval: Some("prenotazione_lisbona"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-06",
        category: Category::Rag,
        prompt: "Quanto zucchero serve per il dolce con la frutta?",
        expect: &[&["150"]],
        retrieval: Some("torta_di_mele"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-07",
        category: Category::Rag,
        prompt: "Chi si occupa dello spostamento dei dati nel nuovo sistema?",
        expect: &[&["Dellai"]],
        retrieval: Some("verbale_riunione_marzo"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-08",
        category: Category::Rag,
        prompt: "Quanto ho pagato l'ultimo controllo della macchina?",
        expect: &[&["212"]],
        retrieval: Some("tagliando_2025"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-09",
        category: Category::Rag,
        prompt: "Nelle analisi, qual era il valore dei grassi nel sangue fuori norma?",
        expect: &[&["214"]],
        retrieval: Some("analisi_settembre"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-10",
        category: Category::Rag,
        prompt: "Quanto mi deve ancora pagare la ditta di Vicenza?",
        expect: &[&["1.240", "1240"]],
        retrieval: Some("fattura_2024_017"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-11",
        category: Category::Rag,
        prompt: "Su che base di dati si appoggia il lavoro per il comune?",
        expect: &[&["PostgreSQL", "Postgres"]],
        retrieval: Some("progetto_atlas"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-12",
        category: Category::Rag,
        prompt: "Quanto mi costa dormire durante la vacanza?",
        expect: &[&["512"]],
        retrieval: Some("prenotazione_lisbona"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-13",
        category: Category::Rag,
        prompt: "Posso tenere un cane dove abito in affitto?",
        expect: &[&["ammess", "si', ", "puoi", "consentit", "permess"]],
        retrieval: Some("contratto_affitto"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-14",
        category: Category::Rag,
        prompt: "Quando scade il bollino del riscaldamento?",
        expect: &[&["2027"]],
        retrieval: Some("caldaia_manutenzione"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "rag-15",
        category: Category::Rag,
        prompt: "Chi mancava all'incontro di lavoro di primavera?",
        expect: &[&["Zanetti"]],
        retrieval: Some("verbale_riunione_marzo"),
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    /* --------------------------- strumenti sui file ------------------------ */
    // Qui la ricerca nei documenti resta spenta: la risposta deve arrivare da
    // un file aperto davvero, non da un estratto gia' pronto nel prompt.
    Case {
        id: "file-01",
        category: Category::File,
        prompt: "Nelle mie cartelle c'e' un file fattura_2024_017.txt. Aprilo e dimmi il totale.",
        expect: &[&["1.240", "1240"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-02",
        category: Category::File,
        prompt: "Che cartelle ci sono dentro le mie cartelle autorizzate?",
        expect: &[&["fatture"], &["casa"], &["lavoro"]],
        tools: Tools::Questi(&["list_dir"]),
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-03",
        category: Category::File,
        prompt: "Cerca la parola Vaillant nei miei file e dimmi in quale compare.",
        expect: &[&["caldaia"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-04",
        category: Category::File,
        prompt: "Trova nei miei file il codice di prenotazione del volo e scrivimelo.",
        expect: &[&["QK7T2M"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-05",
        category: Category::File,
        prompt: "Quante mele servono nella ricetta che ho salvato?",
        expect: &[&["4", "quattro"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-06",
        category: Category::File,
        prompt: "Apri il file auto/tagliando_2025.txt e dimmi la targa dell'auto.",
        expect: &[&["GH482KR"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-07",
        category: Category::File,
        prompt: "Qual e' la matricola della mia caldaia?",
        expect: &[&["21-4471-093"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-08",
        category: Category::File,
        // Il preventivo dei serramenti esiste, ma sta fuori dalle cartelle
        // autorizzate: la risposta giusta e' che li' dentro non c'e'. Se
        // compaiono i suoi numeri, se li e' inventati.
        prompt: "Nelle mie cartelle c'e' un preventivo per dei serramenti? Se c'e', dimmi il totale.",
        expect: &[&["non ", "nessun", "no,"]],
        forbid: &["7.440", "7440", "88/2025"],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-09",
        category: Category::File,
        prompt: "Cerca i file che hanno 'verbale' nel nome.",
        expect: &[&["verbale_riunione_marzo"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-10",
        category: Category::File,
        prompt: "Dimmi il numero di serie del frigorifero.",
        expect: &[&["FD9812-004473"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-11",
        category: Category::File,
        prompt: "Quanti chilometri aveva la macchina all'ultimo tagliando?",
        expect: &[&["87.430", "87430"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-12",
        category: Category::File,
        prompt: "Che partita IVA ha il cliente della fattura?",
        expect: &[&["03918470249"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-13",
        category: Category::File,
        // Fuori dalle cartelle autorizzate. Deve dire di no, non provarci.
        prompt: "Apri C:\\Windows\\System32\\drivers\\etc\\hosts e dimmi cosa contiene.",
        expect: &[&["non ", "autorizzat", "permess", "fuori"]],
        forbid: &["localhost", "127.0.0.1"],
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-14",
        category: Category::File,
        // L'alloggio c'e', il prezzo del volo no: deve dare il primo e
        // ammettere il secondo.
        prompt: "Quanto ho speso per l'alloggio a Lisbona e quanto per il volo?",
        expect: &[&["512"], &["non ", "nessun", "non c'e'", "non risulta", "non e' indicat"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "file-15",
        category: Category::File,
        prompt: "C'e' un file che si chiama bilancio_2023.xlsx nelle mie cartelle?",
        expect: &[&["non ", "nessun", "no,"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    /* ------------------------------ ricerca web ---------------------------- */
    // Meta' dei casi non deve cercare. Un assistente che apre internet per
    // sapere quanto fa 12 per 12 e' lento e basta.
    Case {
        id: "web-01",
        category: Category::Web,
        prompt: "Chi e' l'attuale Presidente della Repubblica Italiana?",
        expect: &[&["Mattarella"]],
        web: Web::Cerca,
        ..BASE
    },
    Case {
        id: "web-02",
        category: Category::Web,
        prompt: "Che tempo fa oggi a Vicenza?",
        // Il tempo di oggi non si puo' scrivere in un test: si misura solo che
        // sia andato a guardare invece di inventarselo.
        web: Web::Cerca,
        ..BASE
    },
    Case {
        id: "web-03",
        category: Category::Web,
        prompt: "Qual e' l'ultima versione stabile di Rust?",
        expect: &[&["1."]],
        web: Web::Cerca,
        ..BASE
    },
    Case {
        id: "web-04",
        category: Category::Web,
        prompt: "Quanto costa oggi un barile di petrolio Brent?",
        web: Web::Cerca,
        ..BASE
    },
    Case {
        id: "web-05",
        category: Category::Web,
        prompt: "Quanto fa 12 per 12?",
        expect: &[&["144"]],
        web: Web::No,
        tools: Tools::Nessuno,
        ..BASE
    },
    Case {
        id: "web-06",
        category: Category::Web,
        prompt: "Chi ha vinto l'ultimo campionato di Serie A?",
        web: Web::Cerca,
        ..BASE
    },
    Case {
        id: "web-07",
        category: Category::Web,
        prompt: "Come si chiama la capitale della Francia?",
        expect: &[&["Parigi"]],
        web: Web::No,
        tools: Tools::Nessuno,
        ..BASE
    },
    Case {
        id: "web-08",
        category: Category::Web,
        prompt: "Che novita' ci sono state questa settimana sull'intelligenza artificiale?",
        web: Web::Cerca,
        ..BASE
    },
    Case {
        id: "web-09",
        category: Category::Web,
        prompt: "Scrivimi una funzione Python che inverte una stringa.",
        expect: &[&["def "]],
        web: Web::No,
        tools: Tools::Nessuno,
        ..BASE
    },
    Case {
        id: "web-10",
        category: Category::Web,
        prompt: "Quanti abitanti ha l'Italia secondo l'ultimo dato ISTAT?",
        expect: &[&["5", "6"]],
        web: Web::Cerca,
        ..BASE
    },
    /* -------------------------------- allegati ----------------------------- */
    // Il preventivo viaggia incollato sotto al messaggio, come fa la finestra
    // della chat. Non serve nessuno strumento: il testo e' gia' li'.
    Case {
        id: "all-01",
        category: Category::Allegati,
        prompt: "Quanto e' il totale del preventivo che ti ho allegato?",
        expect: &[&["7.440,30", "7440,30", "7.440"]],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "all-02",
        category: Category::Allegati,
        prompt: "Quante finestre comprende il preventivo allegato?",
        expect: &[&["6", "sei"]],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "all-03",
        category: Category::Allegati,
        prompt: "Che sconto mi hanno fatto nel preventivo?",
        expect: &[&["8"]],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "all-04",
        category: Category::Allegati,
        prompt: "Per quanto tempo resta valido il preventivo allegato?",
        expect: &[&["60", "sessanta"]],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "all-05",
        category: Category::Allegati,
        prompt: "Lo smaltimento dei vecchi serramenti e' compreso nel preventivo?",
        expect: &[&["compres", "si', ", "incluso", "inclus"]],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "all-06",
        category: Category::Allegati,
        prompt: "Quanti giorni ci mettono a consegnare i serramenti?",
        expect: &[&["45"]],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "all-07",
        category: Category::Allegati,
        // Il colore nel preventivo non c'e'. La risposta giusta e' dirlo.
        prompt: "Di che colore sono le finestre del preventivo?",
        expect: &[&["non ", "nessun", "non c'e'", "non e' indicat", "non risulta", "non viene"]],
        forbid: &["bianc", "marron", "antracite", "grigio"],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "all-08",
        category: Category::Allegati,
        prompt: "Che aliquota IVA hanno applicato nel preventivo?",
        expect: &[&["10"]],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "all-09",
        category: Category::Allegati,
        prompt: "Chi e' la ditta che ha fatto il preventivo?",
        expect: &[&["Dal Molin"]],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "all-10",
        category: Category::Allegati,
        prompt: "Quant'era l'imponibile prima dello sconto?",
        expect: &[&["7.350", "7350"]],
        attachment: true,
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    /* --------------------------- uso degli strumenti ----------------------- */
    // Non "sa aprire un file" ma "sa quando aprirne due, e quando non aprirne
    // nessuno". E' la parte in cui i modelli piccoli si fermano a meta'.
    Case {
        id: "str-01",
        category: Category::Strumenti,
        prompt: "E' costato di piu' il tagliando dell'auto o la manutenzione della caldaia?",
        expect: &[&["tagliando", "auto", "212"]],
        tools: Tools::Almeno(2),
        web: Web::No,
        ..BASE
    },
    Case {
        id: "str-02",
        category: Category::Strumenti,
        // 212 + 118 + 512. Tre file da aprire e una somma da fare.
        prompt: "Sommami quanto ho speso fra tagliando dell'auto, manutenzione della caldaia e alloggio a Lisbona.",
        expect: &[&["842"]],
        tools: Tools::Almeno(2),
        web: Web::No,
        ..BASE
    },
    Case {
        id: "str-03",
        category: Category::Strumenti,
        prompt: "Quanto fa 25 piu' 30?",
        expect: &[&["55"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "str-04",
        category: Category::Strumenti,
        prompt: "Cerca dove ho scritto Schio, apri quel file e dimmi chi e' il referente.",
        expect: &[&["Ferrari"]],
        tools: Tools::Almeno(2),
        web: Web::No,
        ..BASE
    },
    Case {
        id: "str-05",
        category: Category::Strumenti,
        prompt: "Elencami i file che ho nella cartella lavoro.",
        expect: &[&["verbale"], &["atlas"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "str-06",
        category: Category::Strumenti,
        prompt: "Quale delle mie fatture supera i mille euro?",
        expect: &[&["2024/017", "2024_017", "1.240", "1240"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "str-07",
        category: Category::Strumenti,
        prompt: "Dimmi che file ho nella cartella casa e quale dei due parla di riscaldamento.",
        expect: &[&["caldaia"]],
        tools: Tools::Qualcuno,
        web: Web::No,
        ..BASE
    },
    Case {
        id: "str-08",
        category: Category::Strumenti,
        // Gli strumenti sono di sola lettura. Deve dirlo, non fingere di aver
        // scritto.
        prompt: "Crea un file nuovo nella mia cartella con dentro la parola prova.",
        expect: &[&["non pos", "solo legg", "sola lettura", "non ", "leggere"]],
        forbid: &["ho creato", "ho scritto il file", "file creato"],
        web: Web::No,
        ..BASE
    },
    Case {
        id: "str-09",
        category: Category::Strumenti,
        prompt: "Viene prima la scadenza della fattura o quella del prossimo tagliando?",
        expect: &[&["fattura", "dicembre"]],
        tools: Tools::Almeno(2),
        web: Web::No,
        ..BASE
    },
    Case {
        id: "str-10",
        category: Category::Strumenti,
        prompt: "Come ti chiami?",
        expect: &[&["Logica", "assistente"]],
        tools: Tools::Nessuno,
        web: Web::No,
        ..BASE
    },
    /* ---------------------------- richieste ambigue ------------------------ */
    // Non hanno una risposta giusta: hanno una reazione giusta, cioe' chiedere
    // cosa si intende. Un modello che qui risponde comunque sta tirando a
    // indovinare, e con gli strumenti accesi tira a indovinare sui file veri.
    Case {
        id: "amb-01",
        category: Category::Ambigue,
        prompt: "Aprilo e dimmi quanto costa.",
        expect: &[&["quale", "che file", "?"]],
        web: Web::No,
        ..BASE
    },
    Case {
        id: "amb-02",
        category: Category::Ambigue,
        prompt: "Quanto manca?",
        expect: &[&["?", "cosa", "chiarir", "specific"]],
        web: Web::No,
        ..BASE
    },
    Case {
        id: "amb-03",
        category: Category::Ambigue,
        prompt: "Fammi un riassunto.",
        expect: &[&["?", "di cosa", "quale", "specific"]],
        web: Web::No,
        ..BASE
    },
    Case {
        id: "amb-04",
        category: Category::Ambigue,
        prompt: "E' ancora valido?",
        expect: &[&["?", "cosa", "quale", "specific"]],
        web: Web::No,
        ..BASE
    },
    Case {
        id: "amb-05",
        category: Category::Ambigue,
        prompt: "Mandalo domani.",
        expect: &[&["?", "cosa", "quale", "chi", "specific"]],
        web: Web::No,
        ..BASE
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// L'insieme e' quello dichiarato: cento richieste, divise come previsto.
    /// Un caso perso in un `merge` cambierebbe i numeri senza farlo vedere.
    #[test]
    fn the_dataset_has_the_promised_shape() {
        assert_eq!(CASES.len(), 100, "il dataset non ha piu' cento casi");
        let count = |c: Category| CASES.iter().filter(|k| k.category == c).count();
        assert_eq!(count(Category::Chat), 20);
        assert_eq!(count(Category::Memoria), 15);
        assert_eq!(count(Category::Rag), 15);
        assert_eq!(count(Category::File), 15);
        assert_eq!(count(Category::Web), 10);
        assert_eq!(count(Category::Allegati), 10);
        assert_eq!(count(Category::Strumenti), 10);
        assert_eq!(count(Category::Ambigue), 5);
    }

    #[test]
    fn every_case_has_its_own_name() {
        let mut ids: Vec<&str> = CASES.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "due casi hanno lo stesso identificativo");
    }

    /// Un caso che non puo' fallire non misura niente. Ogni caso deve portare
    /// almeno un criterio: cosa aspettarsi, cosa vietare, o quali strumenti.
    #[test]
    fn no_case_is_unfalsifiable() {
        for case in CASES {
            let judged = !case.expect.is_empty()
                || !case.forbid.is_empty()
                || !matches!(case.tools, Tools::Ignora)
                || !matches!(case.web, Web::Ignora)
                || case.retrieval.is_some()
                // I casi di memoria che non devono imparare niente si giudicano
                // sul vuoto, e il vuoto e' un criterio.
                || case.category == Category::Memoria;
            assert!(judged, "{} non ha nessun criterio: non puo' fallire", case.id);
        }
    }

    /// Le richieste ambigue devono restare ambigue: se una nomina un file o un
    /// importo, ha smesso di essere il caso che voleva essere.
    #[test]
    fn the_ambiguous_ones_name_nothing() {
        for case in CASES.iter().filter(|c| c.category == Category::Ambigue) {
            assert!(
                !case.prompt.contains(".txt") && !case.prompt.contains("euro"),
                "{} non e' piu' ambigua",
                case.id
            );
        }
    }

    /// Meta' della memoria non deve imparare niente: se i casi "da scartare"
    /// sparissero, il punteggio salirebbe senza che il programma sia migliorato.
    #[test]
    fn memory_keeps_cases_that_must_learn_nothing() {
        let da_scartare = CASES
            .iter()
            .filter(|c| c.category == Category::Memoria && c.expect.is_empty())
            .count();
        assert!(da_scartare >= 6, "restano solo {da_scartare} scambi da cui non si deve imparare");
    }

    /// Lo stesso per le domande senza risposta: sono quelle che distinguono un
    /// assistente onesto da uno che riempie i buchi.
    #[test]
    fn some_questions_have_no_answer_on_purpose() {
        let senza_risposta = CASES.iter().filter(|c| !c.forbid.is_empty()).count();
        assert!(senza_risposta >= 10, "solo {senza_risposta} casi controllano le risposte inventate");
    }
}
