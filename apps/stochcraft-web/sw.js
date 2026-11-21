// StochCraft's service worker: keeps the app's files so that it opens, and runs models, offline.
//
// Network first: online, every request goes to the network (so a new build is used as soon as
// it is published) and the answer is kept; offline, the kept copy answers. Installing fetches
// the page and every file it names (the hashed WebAssembly and JavaScript), so the app works
// offline from the first visit on. Each site has its own cache, named by its scope: the main
// site, and each pull request's preview under pr/<number>/, which the main site leaves alone.
"use strict";

const CACHE = "stochcraft " + self.registration.scope;
const ALWAYS = [
  "./",
  "./manifest.webmanifest",
  "./icon.svg",
  "./icon-192.png",
  "./icon-512.png",
  "./stochcraft-worker_loader.js",
  "./stochcraft-worker.js",
  "./stochcraft-worker_bg.wasm",
];

// The files a page names: href="./…", import … from './…', module_or_path: './…'.
function named(html) {
  const found = new Set();
  for (const m of html.matchAll(/(?:href=|from |module_or_path: )["'](\.\/[^"']+)["']/g)) {
    found.add(m[1]);
  }
  return [...found];
}

self.addEventListener("install", (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE);
      const page = await fetch("./", { cache: "no-cache" });
      const html = await page.clone().text();
      await cache.put("./", page);
      await cache.addAll(ALWAYS.filter((f) => f !== "./").concat(named(html)));
      await self.skipWaiting();
    })(),
  );
});

// Trunk puts a 16-digit hash in the names of the page's own WebAssembly and JavaScript (and in the
// folder of its snippets); every build has new ones, and the network-first answers kept below
// would pile up. The worker's files (stochcraft-worker*.js/.wasm) are not renamed.
const HASHED = /-[0-9a-f]{16}[._/]/;

// Forgets kept files with a build's hash in their name that the current page no longer names.
async function prune() {
  const cache = await caches.open(CACHE);
  const kept = await cache.match("./");
  if (!kept) {
    return;
  }
  const current = new Set(named(await kept.clone().text()).map((n) => new URL(n, self.registration.scope).href));
  for (const request of await cache.keys()) {
    if (HASHED.test(new URL(request.url).pathname) && !current.has(request.url)) {
      await cache.delete(request);
    }
  }
}

self.addEventListener("activate", (event) => {
  event.waitUntil(prune().catch(() => {}).then(() => self.clients.claim()));
});

// Requests for a preview under this site's pr/ are the preview's own service worker's business.
function mine(url) {
  const scope = new URL(self.registration.scope);
  return url.origin === scope.origin && !url.pathname.startsWith(scope.pathname + "pr/");
}

self.addEventListener("fetch", (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== "GET" || !mine(url)) {
    return;
  }
  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE);
      try {
        const answer = await fetch(request);
        if (answer.ok) {
          await cache.put(request, answer.clone());
        }
        return answer;
      } catch (offline) {
        const kept =
          (await cache.match(request, { ignoreSearch: true })) ||
          (request.mode === "navigate" ? await cache.match("./") : undefined);
        if (kept) {
          return kept;
        }
        throw offline;
      }
    })(),
  );
});
