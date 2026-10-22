# Consent to use documents for testing accuracy: template

_Kone Tshivhinda & Claude, 22 Oct 2026._

> **DRAFT: this has not been reviewed by an attorney.** It is a starting point, written from the
> rules in [ADR-0010](../adr/0010-gold-set-and-evaluation.md) and the Protection of Personal
> Information Act (POPIA). Have a South African attorney (or your POPIA adviser) review and adapt
> it before anyone signs it, and fill in every `[bracket]`. Both the **issuer** and the
> **recipient** of an invoice have an interest in it; get consent from both, or use only
> invoices you issued or received yourself.

**Reference:** C-[number]  **Date:** [date]

## 1. Who is asking

[Name of the operator, registration number, address], called "we" below. Our Information
Officer is [name, email, telephone].

## 2. Who is giving consent

[Name of the person or business, address, contact], called "you" below, in your capacity as
[the issuer / the recipient / both] of the documents listed in section 3.

## 3. The documents

[Describe the documents, for example "about [number] invoices issued by you to your clients
between [date] and [date]", and how they will be provided.]

## 4. What we want to do with them, and nothing else

We want to use copies of these documents to **measure and improve how accurately our software
reads invoices**. Specifically:

1. We remove or replace everything that identifies a person or a business (names, addresses,
   contact details, bank and ID numbers, logos) before the copy is stored with our test
   documents. The amounts, dates and item descriptions are kept so the test is meaningful.
2. A person reads the anonymised copy and records what it says, so the software's reading can
   be compared with it.
3. The anonymised copies are stored **encrypted** on [named device or storage] to which only
   [named people] have access. They are not stored in any public place.
4. [**Only include this paragraph if it applies, and only after the provider's terms have been
   checked:** The anonymised copies may be processed by a language-model service operated by
   [provider], whose servers are outside South Africa. [State what the provider's terms say
   about keeping and using the data, and what safeguards apply.]]
5. We will not sell, publish or share the documents, or use them for any other purpose.

Because the copies are anonymised, we do not expect them to be traceable to you. We treat them
carefully regardless.

## 5. How long we keep them

We keep the anonymised copies until [date], and then delete them. Results of the testing
(counts and percentages) contain no document content and may be kept.

## 6. Your rights

- You may **withdraw this consent at any time** by telling [contact]. We will then delete the
  copies of your documents from the test set, within [number] days, and confirm in writing.
  Results that have already been summarised in counts cannot be separated out.
- You may ask us what we hold, and to correct or delete it.
- You may complain to the Information Regulator ([contact details to be confirmed]).

## 7. Consent

I have read this. I understand it, and I agree that the documents described in section 3 may be
used as set out above. I have the authority to agree for [the business named in section 2].

Name: ____________________  Signature: ____________________  Date: ____________

For us: ____________________  Signature: ____________________  Date: ____________

## After it is signed

- Keep the signed form **apart from the gold set** (the label only holds the reference, C-[number]).
- Put the reference in each case's `consent_ref`, and the deletion date in `retain_until`.
- If consent is withdrawn, delete the cases that carry that reference.
