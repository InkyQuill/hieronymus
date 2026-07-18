drop trigger if exists short_term_memories_ai;
drop trigger if exists short_term_memories_ad;
drop trigger if exists short_term_memories_au;

create trigger short_term_memories_ai after insert on short_term_memories begin
  insert into short_term_memories_fts(rowid, text) values (new.id, new.text);
end;

create trigger short_term_memories_ad after delete on short_term_memories begin
  insert into short_term_memories_fts(short_term_memories_fts, rowid, text)
  values ('delete', old.id, old.text);
end;

create trigger short_term_memories_au after update on short_term_memories begin
  insert into short_term_memories_fts(short_term_memories_fts, rowid, text)
  values ('delete', old.id, old.text);
  insert into short_term_memories_fts(rowid, text) values (new.id, new.text);
end;

drop trigger if exists crystals_ai;
drop trigger if exists crystals_ad;
drop trigger if exists crystals_au;

create trigger crystals_ai after insert on crystals begin
  insert into crystals_fts(rowid, title, text) values (new.id, new.title, new.text);
end;

create trigger crystals_ad after delete on crystals begin
  insert into crystals_fts(crystals_fts, rowid, title, text)
  values ('delete', old.id, old.title, old.text);
end;

create trigger crystals_au after update on crystals begin
  insert into crystals_fts(crystals_fts, rowid, title, text)
  values ('delete', old.id, old.title, old.text);
  insert into crystals_fts(rowid, title, text) values (new.id, new.title, new.text);
end;

insert into short_term_memories_fts(short_term_memories_fts)
select 'rebuild'
where 2 = (
  select count(*) from pragma_table_info('short_term_memories')
  where name in ('id', 'text')
);

insert into crystals_fts(crystals_fts)
select 'rebuild'
where 3 = (
  select count(*) from pragma_table_info('crystals')
  where name in ('id', 'title', 'text')
);
