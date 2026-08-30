/**
 * Il filtro della palette: è l'unica logica del componente, e sbagliarla si
 * nota subito (una voce che non compare mai).
 */
import { describe, expect, it } from "vitest";

import { matchesLabel } from "./CommandPalette";

describe("matchesLabel", () => {
  it("senza query passa tutto", () => {
    expect(matchesLabel("Nuova conversazione", "")).toBe(true);
  });

  it("trova per sottosequenza, come ogni palette", () => {
    expect(matchesLabel("Nuova conversazione", "nco")).toBe(true);
    expect(matchesLabel("Nuova conversazione", "nuova")).toBe(true);
    expect(matchesLabel("Nuova conversazione", "conv")).toBe(true);
  });

  it("ignora maiuscole e spazi scritti nella query", () => {
    expect(matchesLabel("Passa al tema chiaro", "TEMA CH")).toBe(true);
  });

  it("le lettere devono essere in ordine", () => {
    expect(matchesLabel("Impostazioni", "ozzi")).toBe(false);
    expect(matchesLabel("Impostazioni", "xyz")).toBe(false);
  });
});
