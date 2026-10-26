-- Written by Kone Tshivhinda & Claude
-- A page that went to a person can be corrected and confirmed by them; it is then a page that
-- was read, with the new evidence name `person_confirmed` (clearcut-core `ConfidenceBasis`).
-- This only widens two checks; no data changes. Run as the owner role.

alter table page_readings drop constraint page_readings_confidence_basis_check;
alter table page_readings add constraint page_readings_confidence_basis_check
    check (confidence_basis in ('text_confirmed', 'arithmetic_only', 'unresolved', 'person_confirmed'));

alter table page_readings drop constraint page_readings_check;
alter table page_readings add constraint page_readings_check check (
    (state = 'in_progress' and carry is not null and review_reason is null and reading is null
        and rung is null and attempt is null and confidence_basis is null and model is null)
    or (state = 'read' and carry is null and review_reason is null and reading is not null
        and rung is not null and attempt is not null and model is not null
        and confidence_basis in ('text_confirmed', 'arithmetic_only', 'person_confirmed'))
    or (state = 'needs_review' and carry is null and review_reason is not null
        and ((reading is null and rung is null and attempt is null and confidence_basis is null and model is null)
          or (reading is not null and rung is not null and attempt is not null and model is not null
              and confidence_basis = 'unresolved')))
);
