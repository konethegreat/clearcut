-- Written by Kone Tshivhinda & Claude
-- ADR-0006: tenants and documents.

create table tenants (
    id         uuid primary key default gen_random_uuid(),
    name       text not null check (btrim(name) <> ''),
    created_at timestamptz not null default now()
);

create table documents (
    id                uuid primary key default gen_random_uuid(),
    tenant_id         uuid not null references tenants (id),
    original_filename text not null,
    content_type      text not null,
    byte_size         bigint not null check (byte_size >= 0),
    sha256            bytea  not null check (octet_length(sha256) = 32),
    storage_key       text   not null,       -- key in object storage (P1-02), never a path
    page_count        integer check (page_count > 0),
    -- Terminal states only (ADR-0005): a document ends 'done' or 'needs_review'.
    status            text   not null default 'received'
                      check (status in ('received', 'processing', 'done', 'needs_review')),
    created_at        timestamptz not null default now(),
    updated_at        timestamptz not null default now(),
    unique (tenant_id, sha256),              -- de-duplication by content (P2-02)
    unique (tenant_id, id)                   -- target for composite foreign keys
);
