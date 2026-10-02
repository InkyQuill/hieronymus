import {expect,test} from 'bun:test';
import {candidates,select} from './source-span-candidates';
test('exact UTF-8 bytes, repeated passages and CRLF remain distinct',()=>{
  const source='Мира 🌙\r\nчитает.\r\n\r\nミラは読む。\n\nМира 🌙\r\nчитает.';
  const c=candidates(source);expect(c).toHaveLength(3);expect(c[0].text).toBe(c[2].text);expect(c[0].start).not.toBe(c[2].start);
  for(const p of c){expect(Buffer.from(source).subarray(p.start,p.end).toString()).toBe(p.text);expect(select(source,c,p.id)).toEqual(p);}
});
test('any source edit, normalization, forged coordinate or unknown ID invalidates selection',()=>{
  const source='Cafe\u0301\n\nミラ';const c=candidates(source);
  for(const edited of [source+'!',source.normalize('NFC'),'x'+source])expect(()=>select(edited,c,'p0')).toThrow();
  expect(()=>select(source,c.map(p=>({...p,start:p.start+1})),'p0')).toThrow();
  expect(()=>select(source,c,'invented')).toThrow();expect(select(source,c,'no_match')).toBeNull();
});
test('candidate and byte budgets are bounded without rewriting the source',()=>{
  expect(candidates(Array(10).fill('paragraph').join('\n\n'))).toHaveLength(8);
  expect(candidates('x'.repeat(8193))).toEqual([]);
  expect(()=>candidates('x'.repeat(262145))).toThrow();
});
