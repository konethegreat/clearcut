# Written by Kone Tshivhinda & Claude
"""The smallest valid one-page PDF, written to stdout (used by the compose check in CI).

A hand-typed one is rejected by a strict reader because it lacks the cross-reference table, so this
builds it properly.
"""
import sys

objects = [
    b"<</Type/Catalog/Pages 2 0 R>>",
    b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
    b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>",
]
stream = b"BT /F1 12 Tf 20 100 Td (ClearCut check) Tj ET"
objects.append(b"<</Length %d>>\nstream\n%s\nendstream" % (len(stream), stream))
objects.append(b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>")
out = b"%PDF-1.4\n"
offsets = []
for i, body in enumerate(objects, start=1):
    offsets.append(len(out))
    out += b"%d 0 obj\n%s\nendobj\n" % (i, body)
xref = len(out)
out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
for offset in offsets:
    out += b"%010d 00000 n \n" % offset
out += b"trailer\n<</Size %d/Root 1 0 R>>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)
sys.stdout.buffer.write(out)
