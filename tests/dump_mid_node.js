// Escribe .mid de la appweb original (15 géneros × formato 0/1) en el directorio dado.
const fs = require("fs"), vm = require("vm");
let html = fs.readFileSync(__dirname + "/../tools/caja_original.html", "utf8");
html = html.replace('nombre: "Sheker"', 'nombre: "Shaker"');
const js = html.slice(html.indexOf("<script>") + 8, html.lastIndexOf("</script>"));
const ctx = { console }; ctx.globalThis = ctx; vm.createContext(ctx); vm.runInContext(js, ctx);
const C = ctx.CAJA; const dir = process.argv[2]; fs.mkdirSync(dir, { recursive: true });
for (const g of Object.keys(C.PLANTILLAS)) for (const sep of [0, 1]) {
  const mods = {}; const seed = "2026";
  const pat = C.generarPatron({ genero: g, mods, barras: 2, densidad: 50, seed });
  const midi = C.construirMidi(pat, { bpm: 100, swing: 20, human: 14, seed, genero: g, mods, barras: 2, separado: !!sep });
  fs.writeFileSync(`${dir}/${g}-f${sep}.mid`, Buffer.from(midi));
}
