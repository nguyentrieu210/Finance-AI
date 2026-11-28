import {defineConfig} from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import {fileURLToPath} from 'node:url';
const root=fileURLToPath(new URL('.',import.meta.url));
export default defineConfig({
  plugins:[react(),tailwindcss()],
  cacheDir:process.env.FINANCE_DEV_CACHE || 'node_modules/.vite',
  server:{
    host:'127.0.0.1',port:18190,strictPort:true,
    allowedHosts:['namviet.gpcds.site'],
    hmr:{protocol:'wss',host:'namviet.gpcds.site',clientPort:443},
    fs:{strict:true,allow:[root],deny:['.env','.env.*','*.{crt,pem,key}','**/.git/**','**/.*','**/tests/**','**/package-lock.json']},
    cors:false
  }
});
