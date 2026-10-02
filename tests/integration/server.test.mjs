import assert from "node:assert/strict";
import test from "node:test";
import { startFixtureServer } from "./server.mjs";

test("HTTP fixture isolates runs and rejects invalid or oversized reports", async () => {
  const server = await startFixtureServer();
  try {
    server.register("first");
    server.register("second");
    assert.match(server.origin, /^http:\/\/127\.0\.0\.1:\d+$/);
    assert.equal((await fetch(`${server.origin}/case?run=unknown`)).status, 404);
    for (const path of ["case", "blank", "js", "lifecycle"]) {
      assert.equal((await fetch(`${server.origin}/${path}?run=first`)).status, 200);
    }
    const post = (body) => fetch(`${server.origin}/event?run=first`, { method: "POST", body });
    assert.equal((await post(JSON.stringify({ type: "js_ready", preload: true }))).status, 204);
    assert.equal((await post("not json")).status, 400);
    assert.equal((await post(JSON.stringify({ wrong: "shape" }))).status, 400);
    assert.equal((await post("x".repeat(65537))).status, 413);
    const first = await (await fetch(`${server.origin}/state?run=first`)).json();
    const second = await (await fetch(`${server.origin}/state?run=second`)).json();
    assert.equal(first.events.length, 1);
    assert.equal(first.events[0].type, "js_ready");
    assert.deepEqual(second.events, []);
  } finally { await server.close(); }
});
