import { createServer } from "node:http";
import { readFile } from "node:fs/promises";

// The fixture serves only loopback pages and stores short, in-memory test reports.
export async function startFixtureServer() {
  const page = await readFile(new URL("./page.html", import.meta.url));
  const runs = new Map();
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url, "http://127.0.0.1");
      const events = runs.get(url.searchParams.get("run"));
      if (!events) {
        response.writeHead(404).end("Unknown test run");
        return;
      }
      response.setHeader("Cache-Control", "no-store");
      if (request.method === "GET" && ["/case", "/blank"].includes(url.pathname)) {
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
        if (!event || typeof event !== "object" || typeof event.type !== "string") {
          response.writeHead(400).end("Invalid test event");
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
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    register: (id) => runs.set(id, []),
    events: (id) => runs.get(id),
    close: () => new Promise((resolve, reject) => {
      server.close((error) => error ? reject(error) : resolve());
      server.closeAllConnections();
    }),
  };
}
