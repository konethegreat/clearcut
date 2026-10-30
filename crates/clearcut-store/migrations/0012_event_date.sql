-- Written by Kone Tshivhinda & Claude
-- The date a fact belongs to (the invoice date on a reading, not the day it was recorded), copied
-- out of the event body so a period can be selected without reading every body.
--
-- It is text, not a date, because a JSON date is already `YYYY-MM-DD` and that form sorts and
-- compares like a calendar date. A date column would need a cast inside the generated expression,
-- and a cast from text to date is not immutable, so Postgres refuses it in a generated column.

alter table events
    add column event_date text generated always as (body ->> 'date') stored,
    add constraint event_date_is_iso
        check (event_date is null or event_date ~ '^\d{4}-\d{2}-\d{2}$');

create index events_by_event_date on events (tenant_id, event_date)
    where event_date is not null;
