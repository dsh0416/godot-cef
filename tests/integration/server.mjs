import { createServer, get } from "node:http";
import { readFile } from "node:fs/promises";

async function waitUntilReachable(origin) {
  const deadline = Date.now() + 2000;
  for (;;) {
    try {
      await new Promise((resolve, reject) => {
        const request = get(`${origin}/health`, (response) => {
          response.resume();
          if (response.statusCode === 204 && response.headers["x-gdcef-fixture"] === "1") resolve();
          else reject(new Error(`Unexpected fixture health response: ${response.statusCode}`));
        });
        request.setTimeout(250, () => request.destroy(Object.assign(new Error("Fixture health timeout"), { code: "ETIMEDOUT" })));
        request.once("error", reject);
      });
      return;
    } catch (error) {
      if (!["ECONNREFUSED", "ETIMEDOUT"].includes(error.code) || Date.now() >= deadline) throw error;
      // WSL can acknowledge listen before a newly allocated loopback port is
      // reachable. Wait for an actual HTTP handshake, never a fixed startup nap.
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }
}

// Loopback only; fresh, unguessable run IDs isolate each browser process.
export async function startFixtureServer() {
  const page = await readFile(new URL("./page.html", import.meta.url));
  const runs = new Map();
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url, "http://127.0.0.1");
      if (request.method === "GET" && url.pathname === "/health") {
        response.writeHead(204, { "X-Gdcef-Fixture": "1" }).end();
        return;
      }
      const events = runs.get(url.searchParams.get("run"));
      if (!events) {
        response.writeHead(404).end("Unknown test run");
        return;
      }
      response.setHeader("Cache-Control", "no-store");
      if (request.method === "GET" && ["/case", "/blank", "/js", "/lifecycle"].includes(url.pathname)) {
        response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" }).end(page);
      } else if (request.method === "GET" && url.pathname === "/state") {
        response.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify({ events }));
      } else if (request.method === "POST" && url.pathname === "/event") {
        const chunks = [];
        let bytes = 0;
        for await (const chunk of request) {
          bytes += chunk.length;
          if (bytes > 65536) {
            response.writeHead(413).end("Report is too large");
            return;
          }
          chunks.push(chunk);
        }
        const event = JSON.parse(Buffer.concat(chunks).toString("utf8"));
        if (!event || typeof event !== "object" || Array.isArray(event) || typeof event.type !== "string") {
          response.writeHead(400).end("Invalid test event");
          return;
        }
        if (events.length >= 1024) {
          response.writeHead(429).end("Too many test events");
          return;
        }
        events.push({ ...event, received_ms: Date.now() });
        response.writeHead(204).end();
      } else {
        response.writeHead(404).end("Unknown fixture endpoint");
      }
    } catch (error) {
      response.writeHead(400).end(String(error));
    }
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const origin = `http://127.0.0.1:${server.address().port}`;
  try {
    await waitUntilReachable(origin);
  } catch (error) {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
    throw new Error("Loopback fixture did not become reachable", { cause: error });
  }
  return {
    origin,
    register: (id) => runs.set(id, []),
    events: (id) => runs.get(id),
    close: () => new Promise((resolve, reject) => {
      server.close((error) => error ? reject(error) : resolve());
      server.closeAllConnections();
    }),
  };
}
