/** Offline selection prototype. These pointers are never trusted evidence receipts. */
import { createHash } from 'node:crypto';
export type Candidate = { id: string; start: number; end: number; text: string; sourceHash: string };
const hash=(source:string)=>createHash('sha256').update(source).digest('hex');
export function candidates(source:string):Candidate[] {
  if(Buffer.byteLength(source)>262144)throw new Error('Source exceeds prototype bound');
  const result:Candidate[]=[];let bytes=0;
  // Retain Unicode normalization, CRLF and repeated text; IDs depend on position.
  for(const match of source.matchAll(/[^\r\n]+(?:\r?\n(?!\r?\n)[^\r\n]+)*/g)) {
    if(!match[0].trim())continue;
    const length=Buffer.byteLength(match[0]);
    if(result.length===8 || bytes+length>8192)break;
    const start=Buffer.byteLength(source.slice(0,match.index));
    result.push({id:`p${result.length}`,start,end:start+length,text:match[0],sourceHash:hash(source)});bytes+=length;
  }
  return result;
}
export function select(source:string, supplied:Candidate[], id:string):Candidate|null {
  if(id==='no_match')return null;
  // Regenerate the closed set from the current bytes; never trust model offsets.
  const current=candidates(source),old=supplied.find(c=>c.id===id),now=current.find(c=>c.id===id);
  if(!old || !now || old.sourceHash!==now.sourceHash || old.start!==now.start || old.end!==now.end || old.text!==now.text)throw new Error('Unknown, stale or tampered source pointer');
  return now;
}
