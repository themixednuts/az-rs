const CRYENGINE_ORIGIN = "https://www.cryengine.com";

type BrowserRunBinding = {
  quickAction(action: string, payload: Record<string, unknown>): Promise<Response>;
};

type Env = {
  BROWSER: BrowserRunBinding;
};

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value, null, 2), {
    status,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "access-control-allow-origin": "*",
      "access-control-allow-methods": "GET,POST,OPTIONS",
      "access-control-allow-headers": "content-type"
    }
  });
}

function validateCryengineUrl(value: string | null | undefined): string {
  if (!value) {
    throw new Error("missing url");
  }

  const url = new URL(value);
  if (url.origin !== CRYENGINE_ORIGIN) {
    throw new Error(`only ${CRYENGINE_ORIGIN} URLs are allowed`);
  }

  if (!url.pathname.startsWith("/docs/static/engines/cryengine-3/")) {
    throw new Error("only CRYENGINE 3 static documentation URLs are allowed");
  }

  url.hash = "";
  return url.toString();
}

function withCors(response: Response): Response {
  const headers = new Headers(response.headers);
  headers.set("access-control-allow-origin", "*");
  headers.set("access-control-allow-methods", "GET,POST,OPTIONS");
  headers.set("access-control-allow-headers", "content-type");
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers
  });
}

async function quickAction(
  env: Env,
  action: string,
  payload: Record<string, unknown>
): Promise<Response> {
  const response = await env.BROWSER.quickAction(action, payload);
  return withCors(response);
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method === "OPTIONS") {
      return new Response(null, {
        status: 204,
        headers: {
          "access-control-allow-origin": "*",
          "access-control-allow-methods": "GET,POST,OPTIONS",
          "access-control-allow-headers": "content-type"
        }
      });
    }

    const requestUrl = new URL(request.url);

    try {
      if (requestUrl.pathname === "/health") {
        return json({ ok: true });
      }

      if (requestUrl.pathname === "/links" && request.method === "GET") {
        const url = validateCryengineUrl(requestUrl.searchParams.get("url"));
        return await quickAction(env, "links", {
          url,
          visibleLinksOnly: false
        });
      }

      if (requestUrl.pathname === "/markdown" && request.method === "GET") {
        const url = validateCryengineUrl(requestUrl.searchParams.get("url"));
        return await quickAction(env, "markdown", { url });
      }

      if (requestUrl.pathname === "/markdown" && request.method === "POST") {
        const body = await request.json() as { url?: string; html?: string };
        const payload: { url?: string; html?: string } = {};

        if (body.url) {
          payload.url = validateCryengineUrl(body.url);
        }

        if (body.html) {
          payload.html = String(body.html);
        }

        if (!payload.url && !payload.html) {
          return json({ error: "request body must include url or html" }, 400);
        }

        return await quickAction(env, "markdown", payload);
      }

      return json({ error: "not found" }, 404);
    } catch (error) {
      return json({ error: error instanceof Error ? error.message : String(error) }, 400);
    }
  }
};
