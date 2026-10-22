# Anonymisation guide

_Kone Tshivhinda & Claude, 22 Oct 2026. Draft: have it read by whoever advises on POPIA before real documents are collected._

A document enters the gold set only after everything that could identify a person or a
business has been replaced. The aim is a document that **behaves like the original** (same
layout, same kind of values, same arithmetic) but belongs to nobody.

Work on a **copy**. Keep the original where it already was, under its own owner's control.
The anonymised copy is the only thing that goes into the gold folder.

## What to replace

| On the document | Replace with | Notes |
|---|---|---|
| Names of people | a made-up name of the same shape | the same person gets the same new name everywhere in the document |
| Business names (supplier, client) | a made-up business name | keep the legal form: `(Pty) Ltd`, `CC`, `Trust` |
| Street and postal addresses | a plausible address that does not exist | keep the number of lines and the postcode shape |
| Telephone numbers, fax, email, website | fictional ones, or remove | emails: use `example.com` or remove |
| VAT numbers (10 digits, start with 4) | a number of the same shape that you have no reason to think is issued | e.g. `4999900123`; a wrong-looking VAT number is a different test, so keep the shape right |
| Company registration numbers, tax reference numbers | fictional, same shape | |
| Bank names | may stay if it is the bank's own branding | account holder, account number and branch code: **replace or remove** |
| Bank account numbers, card numbers | remove, or fictional of the same length | never leave a real one, even partly |
| ID numbers, passport numbers | remove or fictional | an SA ID number carries a birth date and more |
| Logos and letterheads that identify a business | a neutral grey block | the position and size matter, not the picture |
| Signatures, stamps, handwriting | cover with a neutral mark | unless handwriting is the point of the case: then make it someone else's writing |
| Reference numbers that point to a real transaction (order, account, meter) | fictional, same shape | |

**Keep:** amounts, quantities, dates, currencies, tax treatment, product descriptions that do
not identify anyone. They are the content the system is tested on. If a description names a
person ("Consulting for Mr Dlamini"), change it.

Change amounts only if the amount itself identifies a transaction (a very unusual figure). If
you do, recompute every dependent figure so the document still adds up, unless the arithmetic
error is the point of the case.

## How to do it so it actually works

Hiding text under a black box in a PDF does **not** remove it: the text is still in the file
and can be copied out. Use one of these:

- **Edit the source** (the Word or spreadsheet the invoice came from), then print it to a new
  PDF. Best for synthetic and own documents.
- **A real redaction tool** that deletes the text (not one that only draws over it), then save
  as a new file.
- **Flatten**: print the redacted document to PDF, or render it to images and make a PDF of the
  images. Flattening removes hidden layers, comments, attachments and form fields. (A flattened
  scan has no text layer; tag the case `scan`. That is a different test from a text PDF, so keep
  some of each.)

Also remove what is *around* the page:

- **Metadata**: author, company, title, creation tool, dates, XMP data. (Printing to a new PDF
  usually drops it; check anyway.)
- **Photos**: location and device data (EXIF). Re-save the image without metadata.
- **The file name**: it is replaced by `document.pdf` when you run `new-case`, but check you
  did not put a name in the label either.
- **Pages beyond the invoice**: a cover letter, a remittance slip or terms may carry details.

## Check before you say it is done

Tick all of these for every document. Set `"anonymised": true` in the label only when they
are all true.

- [ ] I searched the whole document for the original names, the supplier's and the client's
      street, the telephone number and the email domain: none appear (use the viewer's search,
      which also searches the hidden text layer).
- [ ] I selected all the text (Ctrl+A), copied it into a plain text editor and read it: nothing
      identifies anyone.
- [ ] No account number, card number or ID number remains.
- [ ] The document's properties (author, title, company) are empty or fictional.
- [ ] Every page was looked at, including the last, footers and watermarks.
- [ ] The arithmetic still works as the original did (or the mistake is the point of the case
      and I tagged it).
- [ ] The label's free text (`name`, `address`, `description`, `terms`) holds the **new**,
      fictional values.
- [ ] `clearcut-gold check` raised no "looks like it holds ..." problem for this case. (It
      looks for ID numbers, email addresses and long numbers in the label text. It cannot see
      names, so it does not replace the checks above.)

## The mapping from old to new

If you keep a note of which fictional name replaced which real one, store it **apart from
the gold set**, encrypted, and delete it as soon as the consent says (or the labelling is
finished, whichever is first). Never put it in the case folder.

## If you find something you cannot remove

Leave the document out. A case that cannot be fully anonymised is not worth the risk.
