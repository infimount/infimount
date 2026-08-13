import http from 'node:http'; import fs from 'node:fs'; import path from 'node:path';
const [root, state]=process.argv.slice(2); if(!root||!state) throw new Error('usage: rehearse-update-server.mjs <asset-dir> <state-file>');
const server=http.createServer((req,res)=>{const rel=decodeURIComponent(new URL(req.url,'http://127.0.0.1').pathname).replace(/^\/+/,''); const file=path.resolve(root,rel); if(!file.startsWith(path.resolve(root)+path.sep)||!fs.existsSync(file)){res.statusCode=404;return res.end('not found');} res.end(fs.readFileSync(file));});
server.listen(0,'127.0.0.1',()=>{const a=server.address(); fs.writeFileSync(state,JSON.stringify({port:a.port,pid:process.pid}));});
process.on('SIGTERM',()=>server.close(()=>process.exit(0)));
