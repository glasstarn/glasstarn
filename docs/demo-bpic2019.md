# Five-minute demo: a real company's purchasing

This demo runs Glasstarn on public data: the BPI Challenge 2019 log,
a year of purchasing at a multinational coatings and paints company (SAP,
anonymised, published under CC BY 4.0). About 250,000 purchase order lines and
1.6 million steps. Everything runs on your computer.

## 1. Download the file (about a minute)

Download `BPI_Challenge_2019.xes` (729 MB) from 4TU.ResearchData:
<https://doi.org/10.4121/uuid:d06aff4b-79f0-45e6-8ec8-e19730c248f1>

## 2. Build the project (about 30 seconds)

```sh
glasstarn workspace demo bpic2019 ~/Downloads/BPI_Challenge_2019.xes
```

This creates the project **BPIC 2019 purchasing (demo)** with the settings a
person would confirm: one case per purchase order line, the order line's own
fields (vendor, spend area, item category, net worth in euros) kept for
comparison, repeated rows kept as real repeated work, and the `batch_` accounts
confirmed as SAP background jobs. On a 2023 laptop it takes about 26 seconds
and a quarter of a gigabyte of memory.

Open it in the browser with `glasstarn serve`, or ask your agent the
questions below. Each answer takes a few seconds.

## 3. Five questions

**1. Where does the time go?**
From recording a vendor's invoice to paying it takes 44 days on average, a
third of all waiting in the process. Next: goods receipt to invoice, 22 days.

**2. How much money is waiting where?**
Order lines worth €997 million in all. €126 million sits on lines whose invoice
is recorded but not yet paid.

**3. What does the usual path look like?**
One line in five goes order, vendor invoice, goods receipt, invoice recorded,
paid. The second most common path has the goods arrive before the invoice. The
top five paths cover 45% of lines.

**4. Where is work repeated?**
"Record Service Entry Sheet" repeats on about 4,100 lines, 159,000 times in
all: service orders confirmed piece by piece.

**5. Do lines invoiced after the goods arrive behave differently?**
Compare the item categories "3-way match, invoice before GR" and "3-way match,
invoice after GR": the second group carries the service entry sheets, and its
lines are worth four times more on average.

## What to notice

Every number links back to the cases behind it, so any finding can be checked
line by line. The slow payment is the company paying its vendors, which may be
deliberate (holding cash until the due date); the data has no due dates, so the
question for the company is whether payments land on time.

Source: van Dongen, B.F. (2019). BPI Challenge 2019. 4TU.ResearchData.
