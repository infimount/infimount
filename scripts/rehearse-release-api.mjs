import fs from 'node:fs'; import path from 'node:path';
const [src,dst]=process.argv.slice(2); if(!src||!dst) throw new Error('usage: rehearse-release-api.mjs <uploaded> <downloaded>');
const files=fs.readdirSync(src).filter(n=>fs.statSync(path.join(src,n)).isFile()).sort(); if(!files.length) throw new Error('no assets');
fs.rmSync(dst,{recursive:true,force:true}); fs.mkdirSync(dst,{recursive:true});
for(const n of files){if(n.includes('/')||n==='.') throw new Error(`invalid asset name: ${n}`); fs.copyFileSync(path.join(src,n),path.join(dst,n));}
const again=fs.readdirSync(dst).filter(n=>fs.statSync(path.join(dst,n)).isFile()).sort();
if(JSON.stringify(files)!==JSON.stringify(again)) throw new Error('round-trip asset manifest mismatch');
for(const n of files){const a=fs.readFileSync(path.join(src,n));const b=fs.readFileSync(path.join(dst,n));if(!a.equals(b)) throw new Error(`round-trip bytes mismatch: ${n}`);}
console.log(`Fake release upload/download round trip passed (${files.length} assets). No network or gh invocation used.`);
