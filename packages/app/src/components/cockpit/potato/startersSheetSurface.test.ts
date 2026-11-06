import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("../../../styles/cockpit-potato-starters.css", import.meta.url), "utf8");
const rule = (selector: string) => css.split("}").find((block) => block.includes(`${selector} {`)) ?? "";
const cockpit = readFileSync(new URL("../../../styles/cockpit.css", import.meta.url), "utf8");

describe("New bot and New group sheets", () => {
  it("sit on a solid surface, not a translucent fill token", () => {
    const sheet = rule(".cockpit .ck-potato-starters");
    expect(sheet).toContain("background: var(--bg-raised)");
    expect(sheet).not.toMatch(/var\(--fill/);
  });
  it("have a dim overlay that is not a translucent fill token", () => {
    const overlay = rule(".cockpit .ck-potato-starters-overlay");
    // The dim is a token (the style rules forbid colour literals in the Potato sheets); the token must still be a dark scrim.
    expect(overlay).toMatch(/background: var\(--scrim\)/);
    expect(cockpit).toMatch(/--scrim:\s*rgba\(0, 0, 0, 0\.[4-9]\);/);
  });
  it("use a surface token that is opaque in both themes", () => {
    const raised = [...cockpit.matchAll(/--bg-raised:\s*([^;]+);/g)].map((m) => m[1]);
    expect(raised.length).toBeGreaterThanOrEqual(2);
    for (const value of raised) expect(value).toMatch(/^#[0-9a-f]{6}$/i);
  });
});
