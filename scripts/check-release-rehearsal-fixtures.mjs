import fs from 'node:fs';
import path from 'node:path';
const root=process.argv[2] || 'tests/fixtures/release-rehearsal';
if (!fs.existsSync(root)) throw new Error(`fixture directory missing: ${root}`);
const forbidden=/BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{20,}/;
for (const file of fs.readdirSync(root,{recursive:true})) { const p=path.join(root,file); if(fs.statSync(p).isFile()){ const b=fs.readFileSync(p); if(b.length>1024*1024) throw new Error(`fixture too large: ${p}`); if(forbidden.test(b.toString('utf8'))) throw new Error(`secret marker in fixture: ${p}`); }}
console.log(`Release rehearsal fixtures passed: ${root}`);
