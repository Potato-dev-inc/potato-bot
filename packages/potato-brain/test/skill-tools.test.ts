import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  createSkill,
  findSkill,
  listSkills,
  MAX_SKILLS,
  removeSkill,
  skillsBlock,
  SKILL_CAP,
  slugOf,
  updateSkill,
  validName,
} from "../src/persona/skills.ts";
import { skillTools } from "../src/core/skill-tools.ts";

const CTX = { turn: 1, where: undefined, model: "m" };

function notesDir(): string {
  return mkdtempSync(join(tmpdir(), "potato-skills-"));
}

function toolNamed(dir: string, name: string) {
  const tool = skillTools(dir).find((t) => t.spec.name === name);
  assert.ok(tool, `${name} is offered`);
  return tool;
}

test("the four skill tools have the right gates: read on owner turns, writes owner's own chats only", () => {
  const dir = notesDir();
  assert.equal(toolNamed(dir, "skill_read").gate, "owner");
  assert.equal(toolNamed(dir, "skill_create").gate, "owner_person");
  assert.equal(toolNamed(dir, "skill_update").gate, "owner_person");
  assert.equal(toolNamed(dir, "skill_delete").gate, "owner_person");
});

test("create writes a skill file in the companion layout and lists it", () => {
  const dir = notesDir();
  const made = createSkill(dir, "Weekly report", "How I like the weekly report", "1. Totals first\n2. Then wins");
  assert.equal(made.ok, true);
  const text = readFileSync(join(dir, "skills", "weekly-report.md"), "utf8");
  assert.equal(text, "# Weekly report\nDescription: How I like the weekly report\n\n1. Totals first\n2. Then wins\n");
  const [only] = listSkills(dir);
  assert.equal(only?.name, "Weekly report");
  assert.equal(only?.description, "How I like the weekly report");
  assert.equal(only?.body, "1. Totals first\n2. Then wins");
});

test("names with path parts or '..' are refused and nothing is written", () => {
  const dir = notesDir();
  for (const bad of ["../../x", "a/b", "a\\b", "..", "x..y"]) {
    assert.ok("error" in validName(bad), `${bad} refused`);
    const r = createSkill(dir, bad, "d", "body");
    assert.equal(r.ok, false);
  }
  assert.equal(existsSync(join(dir, "skills")), false);
});

test("slugs never contain separators and device names get a suffix", () => {
  assert.equal(slugOf("Con"), "con-skill");
  assert.equal(slugOf("../etc"), "etc");
  assert.match(slugOf("你好 world"), /^[\p{L}\p{N}-]+$/u);
  assert.equal(slugOf("!!!"), "");
});

test("a second skill with the same name is refused, and the update tool changes it in place", () => {
  const dir = notesDir();
  createSkill(dir, "Tea", "old", "steps");
  assert.equal(createSkill(dir, "tea", "x", "y").ok, false);
  const changed = updateSkill(dir, "Tea", { body: "new steps" });
  assert.equal(changed.ok, true);
  assert.equal(findSkill(dir, "tea")?.body, "new steps");
  assert.equal(findSkill(dir, "tea")?.description, "old");
});

test("delete moves the file to skills/.removed and keeps it", () => {
  const dir = notesDir();
  createSkill(dir, "Tea", "d", "steps");
  assert.equal(removeSkill(dir, "Tea").ok, true);
  assert.equal(findSkill(dir, "Tea"), null);
  assert.ok(existsSync(join(dir, "skills", ".removed", "tea.md")));
  // a second removal of a same-named skill does not overwrite the first copy
  createSkill(dir, "Tea", "d", "again");
  removeSkill(dir, "Tea");
  assert.ok(existsSync(join(dir, "skills", ".removed", "tea-2.md")));
});

test("the limits hold: body size and skill count", () => {
  const dir = notesDir();
  const r = createSkill(dir, "Big", "d", "x".repeat(SKILL_CAP));
  assert.equal(r.ok, false);
  for (let i = 0; i < MAX_SKILLS; i++) assert.equal(createSkill(dir, `Skill ${i}`, "d", "s").ok, true);
  assert.equal(createSkill(dir, "One more", "d", "s").ok, false);
});

test("skill_read fences the body as data and the prompt block lists names and descriptions only", () => {
  const dir = notesDir();
  createSkill(dir, "Report", "Weekly summary", "Step <one>\nStep two");
  const read = toolNamed(dir, "skill_read").run({ name: "report" }, CTX);
  assert.equal(typeof read, "string");
  assert.match(read as string, /<skill_data tool="skill_read">/);
  assert.match(read as string, /Step &lt;one&gt;/);
  const block = skillsBlock(dir);
  assert.match(block, /owner's notes/);
  assert.match(block, /- Report: Weekly summary/);
  assert.doesNotMatch(block, /Step two/);
});

test("a skill file that does not start with a title is not listed", () => {
  const dir = notesDir();
  mkdirSync(join(dir, "skills"), { recursive: true });
  writeFileSync(join(dir, "skills", "junk.md"), "no title here\n", "utf8");
  assert.deepEqual(listSkills(dir), []);
  assert.equal(skillsBlock(dir), "");
});
