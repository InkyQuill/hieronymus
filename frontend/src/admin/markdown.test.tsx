import { afterEach, describe, expect, it } from "bun:test";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { DetailPane } from "./DetailPane.js";
import { MarkdownBody } from "./markdown.js";

function setupTest() {
  return createOpenTuiHarness({ width: 100, height: 40 });
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("MarkdownBody", () => {
  it("renders common markdown blocks and inline content", async () => {
    const { render, waitForFrame } = setupTest();

    await render(
      <MarkdownBody
        content={[
          "# Crystal Notes",
          "",
          "A **strong** note with *emphasis*, `inline code`, and [source](https://example.test/source).",
          "",
          "- first bullet",
          "- second bullet",
          "1. first step",
          "2. second step",
          "",
          "---",
          "",
          "> quoted memory",
          "",
          "```json",
          '{ "kind": "crystal" }',
          "```",
        ].join("\n")}
      />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Crystal Notes"),
    );

    expect(output).toContain("Crystal Notes");
    expect(output).toContain("strong");
    expect(output).toContain("emphasis");
    expect(output).toContain("inline code");
    expect(output).toContain("source (https://example.test/source)");
    expect(output).toContain("- first bullet");
    expect(output).toContain("- second bullet");
    expect(output).toContain("1. first step");
    expect(output).toContain("2. second step");
    expect(output).toContain("----------------");
    expect(output).toContain("quoted memory");
    expect(output).toContain('{ "kind": "crystal" }');
  });
});

describe("DetailPane markdown body integration", () => {
  it("renders markdown body content through the detail pane", async () => {
    const { render, waitForFrame } = setupTest();

    await render(
      <DetailPane
        detail={{
          title: "Markdown Detail",
          subtitle: "memory",
          body: "## Body Heading\n\n- detail bullet with `code`",
          fields: [],
        }}
      />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Body Heading"),
    );

    expect(output).toContain("Markdown Detail");
    expect(output).toContain("Body Heading");
    expect(output).toContain("- detail bullet with `code`");
  });

  it("keeps JSON and diff body rendering available in the detail pane", async () => {
    const { render, waitForFrame } = setupTest();

    await render(
      <DetailPane
        detail={{
          title: "JSON Detail",
          subtitle: "memory",
          body: '{ "kind": "crystal", "active": true }',
          fields: [],
        }}
      />,
    );

    const jsonOutput = await waitForFrame((frame) => frame.includes("kind"));
    expect(jsonOutput).toContain("crystal");
    expect(jsonOutput).toContain("true");

    await render(
      <DetailPane
        detail={{
          title: "Diff Detail",
          subtitle: "memory",
          body: "diff --git a/file b/file\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new",
          fields: [],
        }}
      />,
    );

    const diffOutput = await waitForFrame((frame) => frame.includes("new"));
    expect(diffOutput).toContain("old");
    expect(diffOutput).toContain("new");
  });
});
