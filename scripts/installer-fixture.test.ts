import {test,expect} from 'bun:test';
import {Database} from 'bun:sqlite';
import {mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {legacyInstallerFixture} from './installer-fixture';
test('installer legacy fixture is a complete markerless Python baseline with preserved author data',()=>{
 const root=mkdtempSync(join(tmpdir(),'hiero-legacy-'));
 try {
  const path=join(root,'hieronymus.sqlite');legacyInstallerFixture(path);
  const db=new Database(path,{readonly:true});
  try {
   expect(db.query("select title from series").get()).toEqual({title:'Preserve this project'});
   expect(db.query("select count(*) as n from sqlite_master where name='hieronymus_meta'").get()).toEqual({n:0});
   expect(db.query("select count(*) as n from sqlite_master where type='table'").get().n).toBeGreaterThan(10);
  } finally {db.close();}
 } finally {rmSync(root,{recursive:true,force:true});}
});
