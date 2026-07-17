import { expect, test } from "bun:test";

test("the web stylesheet defines the data-theme Tailwind dark variant", async () => {
  const css = await Bun.file(new URL("./app.css", import.meta.url)).text();
  expect(css).toContain(
    '@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));',
  );
  expect(css).toContain('@import "tailwindcss";');
});
