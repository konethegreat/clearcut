-- Written by Kone Tshivhinda & Claude
-- ADR-0017: how each page of a document will be read.
--
-- One row per page, written by the analysis stage of the process_document job.
-- It holds facts about the page and the decision made from them, never any of
-- the page's text. The rules that make the decision are in clearcut-core
-- (`page_read`); the checks below repeat the ones that are cheap to state, so a
-- row that contradicts the rules cannot be stored even by a bug.
--
-- Run this as the owner role, never as the application role.

-- ----------------------------------------------------------------- table ----
create table page_reads (
    tenant_id     uuid     not null,
    document_id   uuid     not null,
    page          integer  not null check (page > 0),
    -- Where the text that goes with the page comes from.
    text_source   text     not null check (text_source in ('text_layer', 'ocr', 'none')),
    -- The verdict on the page's text layer (clearcut-core `text_quality`).
    verdict       text     not null check (verdict in ('good', 'needs_check', 'unusable')),
    -- Why, in the fixed vocabulary of `text_quality::Reason`. Never any text.
    reasons       text[]   not null default '{}'
                  check (cardinality(reasons) <= 8
                         and reasons <@ array['no_text', 'replacement_characters', 'ocr_disagrees',
                                              'ocr_partly_agrees', 'ocr_agrees', 'risky_font',
                                              'no_keywords', 'many_vowelless_words']),
    -- Per mille of the text layer's words that OCR also read; null if OCR was not
    -- run, or the page has no text layer to compare with.
    ocr_agreement smallint check (ocr_agreement between 0 and 1000),
    -- How the model will read the page.
    route         text     not null check (route in ('text_and_image', 'image_first', 'image_only')),
    -- The version of the rules that decided (`routing-v1`).
    analysed_with text     not null check (char_length(analysed_with) between 1 and 64),
    created_at    timestamptz not null default now(),
    primary key (document_id, page),
    foreign key (tenant_id, document_id) references documents (tenant_id, id),
    -- The route follows from the verdict, and the text source from the verdict.
    check ((verdict = 'good'        and route = 'text_and_image' and text_source = 'text_layer')
        or (verdict = 'needs_check' and route = 'image_first'    and text_source = 'text_layer')
        or (verdict = 'unusable'    and route = 'image_only'     and text_source in ('ocr', 'none')))
);
create index page_reads_by_document on page_reads (tenant_id, document_id, page);

-- ------------------------------------------------------------ privileges ----
-- The application reads rows, adds them and removes them (a re-run of the stage
-- replaces a document's rows in one transaction). It cannot update one. Unlike
-- events and the audit log these rows are derived and can always be produced
-- again from the file, so they are not append-only.
grant select, insert, delete on page_reads to clearcut_app;

alter table page_reads enable row level security;
create policy app_read   on page_reads for select to clearcut_app using (tenant_id = current_tenant());
create policy app_insert on page_reads for insert to clearcut_app with check (tenant_id = current_tenant());
create policy app_delete on page_reads for delete to clearcut_app using (tenant_id = current_tenant());

-- ------------------------------------------------------------- erasure ----
-- Erasing a document must take its page reads with it (the foreign key would
-- otherwise stop it). The eraser gets only what that needs.
grant select, delete on page_reads to clearcut_eraser;
create policy eraser_read   on page_reads for select to clearcut_eraser using (true);
create policy eraser_delete on page_reads for delete to clearcut_eraser using (true);

-- The same function as in 0005, with one more delete. It keeps its owner, its
-- search_path and its grants (create or replace changes none of them).
create or replace function erase_document(p_tenant uuid, p_document uuid) returns void
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
begin
    if p_tenant is null or p_tenant is distinct from current_tenant() then
        raise exception 'erase_document: tenant does not match the current tenant'
            using errcode = '42501';
    end if;
    if not exists (select 1 from documents where id = p_document and tenant_id = p_tenant) then
        raise exception 'erase_document: document not found' using errcode = 'P0002';
    end if;
    delete from page_reads where document_id = p_document and tenant_id = p_tenant;
    delete from jobs       where document_id = p_document and tenant_id = p_tenant;
    delete from events     where document_id = p_document and tenant_id = p_tenant;
    delete from documents  where id = p_document and tenant_id = p_tenant;
    insert into audit_log (tenant_id, actor, action, subject_type, subject_id, detail)
    values (p_tenant, '{"type":"system"}', 'document.erased', 'document', p_document::text, '{}');
end $$;
