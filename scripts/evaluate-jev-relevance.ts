/** Opt-in pilot using the exact current Rust relevance questions, not the comparison rubric. */
import {constants} from 'node:fs';
import {open} from 'node:fs/promises';
import {createHash} from 'node:crypto';
const [credentials,output]=process.argv.slice(2);
if(!credentials||!output)throw new Error('Usage: bun scripts/evaluate-jev-relevance.ts PRIVATE_RELEVANCE_CONF PRIVATE_OUTPUT');
const source=await Bun.file(new URL('../crates/hiero/src/agent_prompt_delivery/relevance.rs',import.meta.url)).text();
const start=source.indexOf('"questions":{'),end=source.indexOf('\n    });',start);
if(start<0||end<0)throw new Error('Rust question layout changed; inspect rubric extraction before running');
const questions=JSON.parse('{'+source.slice(start,end)+'}').questions;
if(questions.literary?.type!=='noul'||questions.technical?.type!=='noul')throw new Error('Unexpected production rubric');
const config=Bun.TOML.parse(await Bun.file(credentials).text());
if(typeof config.api_key!=='string'||!config.api_key||typeof config.model!=='string'||typeof config.minimum_relevance!=='number'||typeof config.maximum_technical!=='number')throw new Error('Explicit credentials/model/thresholds required');
const f=await Bun.file(new URL('./fixtures/jev/relevance-v1.json',import.meta.url)).json();
if(f.cases.length!==12)throw new Error('Review changed fixture and request budget');
const results=[];const deadline=Date.now()+600000;
for(const mode of ['single','repeat'])for(const c of f.cases){
 if(Date.now()>=deadline)throw new Error('Evaluation deadline exhausted');
 const began=performance.now();const r=await fetch('https://api.typesafe.ai/v1/systemone',{method:'POST',headers:{'content-type':'application/json',authorization:`Bearer ${config.api_key}`},body:JSON.stringify({model:config.model,state:{user_message:c.state.text},questions}),signal:AbortSignal.timeout(Math.min(20000,deadline-Date.now()))});
 if(!r.ok)throw new Error(`Jev HTTP ${r.status}; no retry`);const raw=await r.json();
 const literary=raw.answers?.literary?.noul,technical=raw.answers?.technical?.noul;
 const valid=raw.answers?.literary?.type==='noul'&&raw.answers?.technical?.type==='noul'&&[literary,technical].every(p=>typeof p==='number'&&Number.isFinite(p)&&p>=0&&p<=1);
 results.push({mode,id:c.id,expected:c.expected==='literary',accepted:valid?(literary>=config.minimum_relevance&&technical<=config.maximum_technical):null,requestedModel:config.model,reportedModel:raw.model,latencyMs:performance.now()-began,usage:raw.usage,answers:raw.answers});
 const file=await open(output,constants.O_WRONLY|constants.O_CREAT|constants.O_TRUNC|constants.O_NOFOLLOW,0o600);try{await file.chmod(0o600);await file.writeFile(JSON.stringify({fixture:f.version,rubric:'production-literary-technical-noul-v1',rubricHash:createHash('sha256').update(JSON.stringify(questions)).digest('hex'),thresholds:{minimum_relevance:config.minimum_relevance,maximum_technical:config.maximum_technical},results},null,2)+'\n');}finally{await file.close();}
}
console.log(JSON.stringify({requests:results.length,output}));
