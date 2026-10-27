// Fija la versión de la app antes de compilar en GitHub: node scripts/fijar-version.mjs 0.2.0
// Tauri toma la versión de tauri.conf.json para los instaladores y el «Acerca de».
import { readFileSync, writeFileSync } from 'node:fs';

const version = (process.argv[2] ?? '').replace(/^v/, '');
// El .msi de Windows solo acepta números: X.Y.Z.
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  console.error(`Versión no válida: «${process.argv[2] ?? ''}». Usa el formato 0.2.0`);
  process.exit(1);
}

for (const archivo of ['src-tauri/tauri.conf.json', 'package.json']) {
  const json = JSON.parse(readFileSync(archivo, 'utf8'));
  json.version = version;
  writeFileSync(archivo, JSON.stringify(json, null, 2) + '\n');
}
console.log(`Versión ${version}`);
