/**
 * `engine` è una stringa sola che viaggia fino al file della chat: qui si
 * controlla che si sciolga sempre nei due campi giusti, e soprattutto che un
 * valore che non si riconosce non rompa niente.
 */
import { describe, expect, it } from "vitest";

import { DEFAULT_ENGINE, engineLabelOf, engineOverride } from "./types";

const settings = { use_premium_ai: false, ollama_model: "llama3.1:8b" };

describe("engineOverride", () => {
  it("senza scelta vale quello delle impostazioni", () => {
    expect(engineOverride(DEFAULT_ENGINE, settings)).toEqual(settings);
    expect(engineOverride(undefined, settings)).toEqual(settings);
  });

  it("claude accende il motore a pagamento", () => {
    expect(engineOverride("claude", settings).use_premium_ai).toBe(true);
  });

  it("un modello Ollama spegne Claude anche quando le impostazioni lo vogliono", () => {
    const premium = { use_premium_ai: true, ollama_model: "llama3.1:8b" };
    expect(engineOverride("ollama:qwen2.5:14b", premium)).toEqual({
      use_premium_ai: false,
      // Il nome del modello contiene i due punti: si taglia solo il prefisso.
      ollama_model: "qwen2.5:14b",
    });
  });

  it("un valore sconosciuto vale come 'usa le impostazioni'", () => {
    // Una chat scritta da una versione futura non deve rompere questa.
    expect(engineOverride("motore-che-non-esiste", settings)).toEqual(settings);
  });
});

describe("engineLabelOf", () => {
  it("dice il nome del modello che risponderà davvero", () => {
    expect(engineLabelOf(DEFAULT_ENGINE, settings)).toBe("llama3.1:8b");
    expect(engineLabelOf("claude", settings)).toBe("claude-api");
    expect(engineLabelOf("ollama:mistral", settings)).toBe("mistral");
  });
});
