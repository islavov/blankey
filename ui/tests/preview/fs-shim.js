// node:fs for the fake backend in a browser: it only reads the language catalogs.
export function readFileSync(url) {
  const request = new XMLHttpRequest();
  request.open("GET", String(url), false);
  request.send();
  if (request.status !== 200) throw new Error(`${url}: ${request.status}`);
  return request.responseText;
}
