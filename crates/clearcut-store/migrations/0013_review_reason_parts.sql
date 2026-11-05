-- Written by Kone Tshivhinda & Claude
-- A page read in parts can end with one of three new review reasons: `part_conflict`,
-- `part_failed`, `too_dense` (clearcut-core `ReviewReason`). This only widens one check;
-- no data changes. Run as the owner role.

alter table page_readings drop constraint page_readings_review_reason_check;
alter table page_readings add constraint page_readings_review_reason_check
    check (review_reason in (
        'top_rung_failed', 'too_long', 'refused', 'budget', 'call_cap',
        'cannot_be_sent_safely', 'request_rejected', 'no_rung_available',
        'amounts_include_vat', 'tax_not_shown',
        'part_conflict', 'part_failed', 'too_dense'));
