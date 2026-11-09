// Copies the chosen logos from @web3icons/core (MIT) into src/icons.ts.
// Run: npm i --no-save @web3icons/core@4.0.40 && node scripts/gen-icons.mjs \
//   "$PWD/node_modules/@web3icons/core/dist/svgs" src/icons.ts
import fs from 'node:fs';
const [dir, out] = process.argv.slice(2);
const pick = {
  'chain-near': 'networks/branded/near-protocol', 'chain-84532': 'networks/branded/base', 'chain-11155111': 'networks/branded/ethereum',
  'chain-421614': 'networks/branded/arbitrum-one', 'chain-11155420': 'networks/branded/optimism', 'chain-97': 'networks/branded/binance-smart-chain',
  'chain-80002': 'networks/branded/polygon', 'chain-43113': 'networks/branded/avalanche', 'chain-10143': 'networks/branded/monad',
  'chain-998': 'networks/branded/hyper-evm', 'chain-bitcoin': 'networks/branded/bitcoin', 'chain-solana': 'networks/branded/solana',
  'chain-tron': 'networks/branded/tron', 'chain-xrp': 'networks/branded/xrp', 'chain-cardano': 'networks/branded/cardano',
  'chain-litecoin': 'networks/branded/litecoin', 'chain-dogecoin': 'tokens/branded/DOGE', 'chain-bitcoin-cash': 'tokens/branded/BCH',
  'chain-zcash': 'tokens/branded/ZEC', USDC: 'tokens/branded/USDC',
};
let src =
  '// Chain and token logos from @web3icons/core 4.0.40 (MIT License,\n' +
  '// https://github.com/0xa3k5/web3icons), copied here so the page loads no\n' +
  '// third-party code. Generated; keyed by dashboard row id (and token symbol).\n' +
  'export const ICONS: Record<string, string> = {\n';
for (const [k, f] of Object.entries(pick)) {
  const mod = await import(`${dir}/${f}.svg.js`);
  let svg = String(mod.default).replace(/\n\s*/g, '');
  if (!svg.startsWith('<svg')) throw new Error(f);
  // XRP's mark is white (for dark backgrounds): draw it in the text colour.
  if (k === 'chain-xrp') svg = svg.replace(/fill="#fff"/gi, 'fill="currentColor"');
  if (/<script|\son[a-z]+=|href=/i.test(svg)) throw new Error(`unexpected content in ${f}`);
  // Inline SVGs share one id space: prefix each logo's ids.
  const pre = 'i' + k.replace(/[^a-z0-9]/gi, '');
  svg = svg.replace(/id="([^"]+)"/g, `id="${pre}-$1"`).replace(/url\(#([^)]+)\)/g, `url(#${pre}-$1)`);
  src += `  ${JSON.stringify(k)}: ${JSON.stringify(svg)},\n`;
}
// Dogecoin's logo is a detailed illustration that reads poorly at 24 px: use
// its currency sign, Ð, on its gold (#C2A633) instead.
src = src.replace(/  "chain-dogecoin": .*\n/, '  "chain-dogecoin": ' + JSON.stringify('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><circle cx="12" cy="12" r="11" fill="#C2A633"/><path fill="#fff" d="M9 6.5h3.4c3.2 0 5.1 2.1 5.1 5.5s-1.9 5.5-5.1 5.5H9v-4.6H7.6v-1.8H9zm2.2 2v2.6h2.1v1.8h-2.1v2.6h1.2c1.9 0 2.8-1.2 2.8-3.5s-.9-3.5-2.8-3.5z"/></svg>') + ',\n');
fs.writeFileSync(out, src + '};\n');
console.log('wrote', Object.keys(pick).length, 'icons');
