## Worked examples

These examples are invented. They show the format only; the document you are given is a different one.

### Example 1: the first page of a tax invoice

<page_text id="example" source="text_layer">
Acme Consulting (Pty) Ltd
12 Rivonia Road, Sandton, 2196
VAT Reg No: 4123456789
TAX INVOICE
Invoice No: INV-2041
Date: 03/04/2026
Bill to: Blue Sky Traders (Pty) Ltd, 8 Long Street, Cape Town, 8001, VAT No 4987654321
Description              Qty   Unit price (excl)   Amount (excl)
Consulting - April        2    R 1 500,00          R 3 000,00
Travel (zero-rated)       1    R 250,00            R 250,00
Subtotal                                           R 3 250,00
VAT 15%                                            R 450,00
Total due                                          R 3 700,00
Due date: 03/05/2026   Terms: 30 days
</page_text id="example">

Correct output:

```json
{
  "currency": "ZAR",
  "amounts_include_vat": "no",
  "invoice_number": "INV-2041",
  "invoice_date": "2026-04-03",
  "supplier": { "name": "Acme Consulting (Pty) Ltd", "address": "12 Rivonia Road, Sandton, 2196", "vat_number": "4123456789" },
  "recipient": { "name": "Blue Sky Traders (Pty) Ltd", "address": "8 Long Street, Cape Town, 8001", "vat_number": "4987654321" },
  "lines": [
    { "description": "Consulting - April", "quantity": "2", "unit_price": "1500.00", "line_total": "3000.00", "tax": "standard" },
    { "description": "Travel (zero-rated)", "quantity": "1", "unit_price": "250.00", "line_total": "250.00", "tax": "zero_rated" }
  ],
  "totals": { "net": "3250.00", "vat": "450.00", "gross": "3700.00" },
  "carried": { "brought_forward": "", "carried_forward": "" },
  "due": { "date": "2026-05-03", "terms": "30 days" },
  "credit_note": { "original_invoice_number": "", "amount": "" }
}
```

### Example 2: a till slip whose prices include VAT

<page_text id="example" source="ocr">
QUICK MART
Shop 4, Main Road, Paarl, 7646
VAT No 4555666777
TAX INVOICE
Slip 0045123    2026-04-11
2 x Bread          14,99     29,98
1 x Milk 1l        22,50     22,50
All prices include VAT
Total excl VAT              45,63
VAT 15%                      6,85
TOTAL                R 52,48
</page_text id="example">

Correct output (the amounts are written as printed; the page says they include VAT):

```json
{
  "currency": "ZAR",
  "amounts_include_vat": "yes",
  "invoice_number": "0045123",
  "invoice_date": "2026-04-11",
  "supplier": { "name": "Quick Mart", "address": "Shop 4, Main Road, Paarl, 7646", "vat_number": "4555666777" },
  "recipient": { "name": "", "address": "", "vat_number": "" },
  "lines": [
    { "description": "Bread", "quantity": "2", "unit_price": "14.99", "line_total": "29.98", "tax": "standard" },
    { "description": "Milk 1l", "quantity": "1", "unit_price": "22.50", "line_total": "22.50", "tax": "standard" }
  ],
  "totals": { "net": "45.63", "vat": "6.85", "gross": "52.48" },
  "carried": { "brought_forward": "", "carried_forward": "" },
  "due": { "date": "", "terms": "" },
  "credit_note": { "original_invoice_number": "", "amount": "" }
}
```

### Example 3: a continuation page that carries a balance forward

<page_text id="example" source="text_layer">
Page 2 of 3
Brought forward                              5 000,00
Description          Qty   Unit price   Amount   VAT
Hosting - April       1    800,00       800,00    S
Support retainer      1    1 200,00     1 200,00  S
Subtotal                                     2 000,00
VAT 15%                                        300,00
Carried forward                              7 300,00
</page_text id="example">

Correct output (nothing from the first page is repeated; the page carries 5 000,00 in and 7 300,00 out, which is not the invoice's own total):

```json
{
  "currency": "not_shown",
  "amounts_include_vat": "not_shown",
  "invoice_number": "",
  "invoice_date": "",
  "supplier": { "name": "", "address": "", "vat_number": "" },
  "recipient": { "name": "", "address": "", "vat_number": "" },
  "lines": [
    { "description": "Hosting - April", "quantity": "1", "unit_price": "800.00", "line_total": "800.00", "tax": "standard" },
    { "description": "Support retainer", "quantity": "1", "unit_price": "1200.00", "line_total": "1200.00", "tax": "standard" }
  ],
  "totals": { "net": "2000.00", "vat": "300.00", "gross": "2300.00" },
  "carried": { "brought_forward": "5000.00", "carried_forward": "7300.00" },
  "due": { "date": "", "terms": "" },
  "credit_note": { "original_invoice_number": "", "amount": "" }
}
```
