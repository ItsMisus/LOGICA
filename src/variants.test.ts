/**
 * I rami di una risposta. La proprietà che conta è che `content` resti sempre
 * una copia dell'alternativa attiva: il file della chat deve leggersi anche
 * senza sapere niente dei rami.
 */
import { describe, expect, it } from "vitest";

import { addVariant, withVariant, type ChatMessage } from "./types";

function answer(text: string): ChatMessage {
  return { id: "m1", role: "ai", content: text, timestamp: "2026-08-30T10:00:00Z", type: "text" };
}

describe("addVariant", () => {
  it("la prima rigenerazione fa due alternative, non una", () => {
    // Una risposta scritta prima che esistessero i rami non ha `variants`: la
    // sua diventa la prima.
    const grown = addVariant(answer("prima"), { content: "seconda", timestamp: "t2" });
    expect(grown.variants).toHaveLength(2);
    expect(grown.variants?.[0].content).toBe("prima");
    expect(grown.variant).toBe(1);
    expect(grown.content).toBe("seconda");
  });

  it("rigenerando ancora si accumula", () => {
    let m = addVariant(answer("uno"), { content: "due", timestamp: "t2" });
    m = addVariant(m, { content: "tre", timestamp: "t3" });
    expect(m.variants?.map((v) => v.content)).toEqual(["uno", "due", "tre"]);
    expect(m.variant).toBe(2);
    expect(m.content).toBe("tre");
  });

  it("porta con sé fonti e strumenti dell'alternativa", () => {
    const m = addVariant(answer("uno"), {
      content: "due",
      timestamp: "t2",
      sources: [{ title: "T", url: "https://esempio.it", snippet: "" }],
    });
    expect(m.sources).toHaveLength(1);
  });
});

describe("withVariant", () => {
  it("tornando indietro tornano anche fonti e strumenti di quella risposta", () => {
    const first = answer("uno");
    first.sources = [{ title: "T", url: "https://esempio.it", snippet: "" }];
    const m = withVariant(addVariant(first, { content: "due", timestamp: "t2" }), 0);

    expect(m.content).toBe("uno");
    expect(m.variant).toBe(0);
    expect(m.sources).toHaveLength(1);
    // Le alternative restano tutte: navigare non ne butta via nessuna.
    expect(m.variants).toHaveLength(2);
  });

  it("un indice che non esiste lascia il messaggio com'è", () => {
    const m = answer("uno");
    expect(withVariant(m, 5)).toBe(m);
  });
});
