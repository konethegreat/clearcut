# The gold set: how to build it

_Kone Tshivhinda & Claude, 22 Oct 2026. Part of [ADR-0010](../adr/0010-gold-set-and-evaluation.md)._

The gold set is a collection of real, **anonymised** South African invoices, each with a
hand-written label saying exactly what is on it. Everything the product claims about accuracy
is measured against it: prompts, models, confidence thresholds and the release gate. The
release-blocking target is **zero silent critical errors** (a wrong, missing or invented
critical value that nobody was told about).

It is the long pole of Phase 2, and it depends on people and consent more than on code.

## The rules in one minute

1. **Real documents never go in this repository.** It is public. They live in a private,
   encrypted folder outside the working copy (for example `D:\clearcut-gold\` on an encrypted
   drive). The repository holds only the tools and a synthetic sample set.
2. **Only three sources are allowed:** documents the operator of ClearCut owns, documents
   whose issuer **and** recipient gave written consent, and synthetic or properly licensed
   samples. Nothing scraped, nothing "found".
3. **Anonymise before a document enters the set**, then say so in the label. See the
   [anonymisation guide](anonymisation-guide.md).
4. **No real invoice is sent to any outside service** (including any language model) until the
   provider's retention terms are verified and the privacy work (P2-19) is done. Until then,
   evaluation runs on synthetic and the operator's own non-sensitive documents only.
5. **Reports never show what a document says.** They show case ids, field names and counts.

## What a gold set looks like

```text
D:\clearcut-gold\
  g-0001\
    document.pdf      the anonymised file, exactly as it would be uploaded
    label.json        what is on it (the truth), who labelled it, where it came from
  g-0002\ ...
```

Case ids are opaque (`g-0001`). Nothing in a folder name or file name may identify a person,
a supplier or a client.

## The workflow, per document

| Step | Who | How |
|---|---|---|
| 1. Get consent (unless the operator's own) | Kone | [consent form](consent-form-template.md), keep the signed copy apart from the gold set; note its reference |
| 2. Anonymise | labeller | [anonymisation guide](anonymisation-guide.md); work on a copy, never the original |
| 3. Start the case | labeller | `clearcut-gold new-case D:\clearcut-gold anonymised.pdf --split dev --tier consented --labelled-by KT --pages 2 --consent-ref C-017 --retain-until 2031-03-31` |
| 4. Label it | labeller | fill in `label.json` by the [labelling guide](labelling-guide.md); set `"anonymised": true` once step 2 is done |
| 5. Check | labeller | `clearcut-gold check D:\clearcut-gold` until it says "no problems" |
| 6. Second look | reviewer | check every critical field of at least 1 case in 5; put your initials in `reviewed_by` |

The `--split` choice matters: **all documents from one supplier go in the same split**
(`check` enforces it), about 70% dev and 30% test. Prompts and thresholds are tuned on
`dev`. `test` is the release gate and is run sparingly; **label test cases from scratch**, never
pre-filled by a model, or the score would favour the model that wrote the draft.

## The tools

```text
clearcut-gold new-case <gold> <document> --split --tier --labelled-by --pages [...]
clearcut-gold check    <gold>                       is the set well formed and safe?
clearcut-gold score    <gold> <results> [--split]   score a run, apply the release gate
clearcut-gold guard    <repository>                 no documents or labels in the repository
clearcut-gold sample   <new folder>                 the synthetic sample set
```

Run them with `cargo run -p clearcut-eval -- <command>` from the repository, or build once with
`cargo build --release -p clearcut-eval` and use `target\release\clearcut-gold.exe`.

## The guard

CI runs `clearcut-gold guard .` on every pull request. It fails if a PDF, image, Office file,
archive or email, or any gold-set `label.json`, is in the repository outside the approved
fixture folders listed in [`gold-guard.json`](../../gold-guard.json), or if `.gitignore` no
longer keeps `/gold/` and `/gold-*/` out. Adding an approved folder means editing that file in
a pull request, so it is always a visible decision. (A secrets scanner cannot see a PDF; this
can.)

## What is still open

- Where the private folder lives, and who may open it.
- A first source of real documents, and who gives consent for them.
- The consent form needs review by a South African attorney before it is used.
- Who is the Information Officer for the operator (POPIA).
- The questions at the end of the [labelling guide](labelling-guide.md).
