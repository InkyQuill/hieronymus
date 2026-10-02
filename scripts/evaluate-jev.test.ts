import { describe, expect, test } from 'bun:test';
import { decision } from './evaluate-jev';
const answer = {type:'choice',choice:'equivalent',confidence:.99,probabilities:{equivalent:.99,distinct:.01,contradictory:0,insufficient_context:0}};
describe('evaluation acceptance matches conservative production parser', () => {
  test('accepts a complete confident answer', () => expect(decision(answer)).toBe('equivalent'));
  test('abstains on malformed, uncertain or inconsistent answers', () => {
    for (const value of [null,{}, {...answer,confidence:.94}, {...answer,choice:'unknown'}, {...answer,probabilities:{equivalent:1}}, {...answer,probabilities:{...answer.probabilities,distinct:.4}}, {...answer,confidence:NaN}]) expect(decision(value)).toBe('insufficient_context');
  });
});
