import {expect,test} from 'bun:test';
import {acceptedChoice,validateFixture} from './evaluate-jev-pilot';
test('closed-set answers require complete confident distributions',()=>{
  const a={type:'choice',choice:'yes',confidence:1,probabilities:{yes:1,no:0}};
  expect(acceptedChoice(a,['yes','no'])).toBe('yes');
  for(const b of [{...a,choice:'invented'}, {...a,confidence:.94}, {...a,probabilities:{yes:1}}, {...a,probabilities:{yes:.9,no:.2}},null])expect(acceptedChoice(b,['yes','no'])).toBeNull();
});
test('fixture rejects unknown gold and duplicate identities',()=>{
  const f={version:'v1',instructions:'test',criteria:{yes:'yes',no:'no'},cases:[{id:'a',language:'en',state:{},expected:'yes'}]};
  expect(validateFixture(f)).toBe(f);
  expect(()=>validateFixture({...f,cases:[...f.cases,...f.cases]})).toThrow();
  expect(()=>validateFixture({...f,cases:[{...f.cases[0],expected:'other'}]})).toThrow();
});
