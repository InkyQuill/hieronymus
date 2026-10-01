#!/usr/bin/env bun
import {Database} from "bun:sqlite";
import {readFileSync} from "node:fs";
export function legacyInstallerFixture(path:string) {
  const db=new Database(path,{create:true});
  try {
    db.exec(readFileSync(new URL('../crates/hieronymus/migrations/global.sql',import.meta.url),'utf8'));
    db.exec("insert into series(slug,title,default_source_language,default_target_language,created_at,updated_at) values('installer-fixture','Preserve this project','ja','ru','2026-01-01','2026-01-01')");
  } finally {db.close();}
}
if(import.meta.main)legacyInstallerFixture(Bun.argv[2]);
