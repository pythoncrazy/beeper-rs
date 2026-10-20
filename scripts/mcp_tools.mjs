// Copies the MCP tools of Beeper Desktop, with the API route of each, to crates/beeper-api/assets/mcp_tools.json.
//
// Usage: node scripts/mcp_tools.mjs <the extracted app.asar of Beeper Desktop>
import fs from "node:fs";

const dir = `${process.argv[2]}/build/main`;
const file = fs.readdirSync(dir).find((f) => /^http-.*\.mjs$/.test(f) && fs.readFileSync(`${dir}/${f}`, "utf8").includes('name:"search_docs"'));
const s = fs.readFileSync(`${dir}/${file}`, "utf8");
const order = [...s.matchAll(/\$t\((\w+)\);/g)].map((m) => m[1]);

function literal(at) {
  let depth = 0, i = at, quote = null;
  for (; i < s.length; i++) {
    const c = s[i];
    if (quote) {
      if (c === "\\") i++;
      else if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "{") depth++;
    else if (c === "}" && --depth === 0) break;
  }
  return eval(`(${s.slice(at, i + 1)})`);
}

const tools = order.map((entry) => {
  const at = s.indexOf(`${entry}={metadata:`);
  const t = s.lastIndexOf("{name:", s.lastIndexOf("annotations:", at));
  const meta = literal(s.lastIndexOf("{resource:", t));
  const text = s.slice(t, at).includes(".text()") ? { text: true } : {};
  return { method: meta.httpMethod.toUpperCase(), path: meta.httpPath, ...text, tool: literal(t) };
});
const docs = s.indexOf('{name:"search_docs",description:');
const url = s.slice(docs).match(/DOCS_SEARCH_URL\|\|"([^"]+)"/)[1];
tools.push({ method: "GET", path: url, tool: literal(docs) });
fs.writeFileSync("crates/beeper-api/assets/mcp_tools.json", JSON.stringify(tools, null, 1) + "\n");
