import { createRequire } from "node:module";
import fs from "node:fs/promises";
import path from "node:path";
const require = createRequire(
  "/home/papan/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright/package.json",
);
const { chromium } = require("playwright");
const root = path.resolve(import.meta.dirname, "..");
const browser = await chromium.launch({
  executablePath: "/usr/bin/google-chrome",
  headless: true,
  args: ["--no-sandbox"],
});
const page = await browser.newPage({
  viewport: { width: 1440, height: 900 },
  deviceScaleFactor: 1,
});
await page.addInitScript(() => {
  localStorage.setItem(
    "reflow.onboarding.v1",
    JSON.stringify({ version: 1, stage: 5, outcome: "completed" }),
  );
  localStorage.setItem("reflow.appearance", JSON.stringify({ app_theme: "light" }));
});
await page.route("**/src/services/tauriApi.ts*", async (route) => {
  const response = await route.fetch();
  const source = await response.text();
  const seed = `
const promoApi=api;
const baseSettings=promoApi.getSettings;
promoApi.getSettings=async()=>({...await baseSettings(),app_theme:'light',reduce_motion:true,hotkey:'Ctrl+Shift+Space',dictionary_terms:[{id:'1',term:'Reflow',preferred_spelling:'Reflow',category:'Custom'},{id:'2',term:'Tauri',preferred_spelling:'Tauri',category:'Custom'}],custom_replacements:[{id:'1',before:'my signature',after:'Best, Alex',enabled:true}]});
promoApi.getModelStatus=async()=>({installed:true,loaded:true,version:'0.6B-v1',name:'Qwen3-ASR',size_bytes:1880000000,download_progress_pct:100,download_speed_mbps:0,backend:'CPU',is_downloading:false,is_loading:false,error:null});
promoApi.getIntelligenceTiers=async()=>[{tier:'smart_flow',model_id:'qwen3.5-0.8b',installed:true,weights_installed:true,downloading:false}];
const samples=[
['Launch plan','The launch plan is ready. Let’s review the final details together tomorrow morning.','Email','dictation'],
['A thought for the next chapter','Make more space for the ideas that arrive when you stop watching the cursor.','Notes','note'],
['Project update','The first draft is complete. Next up: review the feedback, refine the details, and share the update.','Notes','note'],
['Quick message','Sounds great. I’ll send you the updated version this afternoon.','Chat','dictation']
].map((r,i)=>({id:String(i),kind:r[3],source:'manual',pinned:i===1,tags:i===1?'ideas':'',created_at:new Date(Date.now()-i*3600000).toISOString(),duration_ms:13000,language:'en',raw_transcript:r[1],final_transcript:r[1],application_name:r[2],application_process:r[2],word_count:r[1].split(' ').length,character_count:r[1].length,model_version:'Qwen3-ASR',processing_mode:'smart'}));
promoApi.getHistory=async(limit=50)=>samples.slice(0,limit);
promoApi.queryHistory=async(q)=>{const entries=samples.filter(x=>!q.kind||x.kind===q.kind);return{entries,total:entries.length,next_offset:null,recovery_notice:null}};
`;
  await route.fulfill({ response, body: source + "\n" + seed });
});
page.on("pageerror", (e) => console.log("PAGEERROR", e.message));
await page.goto("http://127.0.0.1:5178/", { waitUntil: "networkidle" });
await page.evaluate(() => document.documentElement.classList.remove("dark"));
await page.waitForTimeout(800);
await page.screenshot({ path: path.join(root, "assets/home-empty.png") });
await page
  .getByRole("textbox", { name: "Transcript", exact: true })
  .fill("The launch plan is ready. Let’s review the final details together tomorrow morning.");
await page.getByRole("textbox", { name: "Transcript", exact: true }).blur();
await page.screenshot({ path: path.join(root, "assets/home.png") });
await page.getByRole("button", { name: "Notes", exact: true }).click();
await page.waitForTimeout(500);
await page.screenshot({ path: path.join(root, "assets/notes.png") });
await page.getByRole("button", { name: "History", exact: true }).click();
await page.waitForTimeout(500);
await page.screenshot({ path: path.join(root, "assets/history.png") });
await page.getByRole("button", { name: "Settings", exact: true }).click();
await page.waitForTimeout(500);
await page.screenshot({ path: path.join(root, "assets/settings.png") });
const dictionary = page.getByRole("button", { name: "Dictionary", exact: true });
if (await dictionary.count()) {
  await dictionary.click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(root, "assets/dictionary.png") });
}
console.log("Product screenshots saved:", (await fs.readdir(path.join(root, "assets"))).join(", "));
await browser.close();
/* global localStorage, document, console */
