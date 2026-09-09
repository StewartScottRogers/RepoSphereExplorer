# LogDigest

Summarises web server logs: parse the lines, count what happened, and
report rates over a window.

## Using it

```perl
use LogDigest;

my $digest = LogDigest->new;
$digest->parse_handle($log);

printf "%d entries, %.1f ms average
", $digest->count, $digest->average_millis;
print $digest->report;

my %by_class = $digest->by_status_class;
my @slowest  = $digest->slowest(5);
```

## Notes

- `parse_line` returns nothing for a line it does not recognise, rather
  than guessing at the fields. A log summary built from misparsed lines is
  worse than no summary.
- `use strict` and `use warnings` are enforced at severity 5 by
  `.perlcriticrc`; everything else is advisory.
- Counts are kept as plain hashes rather than objects, because the whole
  module is a few hundred lines and an object per counter would be more
  ceremony than code.

## Developing

```bash
cpanm --installdeps .
prove -lv t/
perlcritic lib t
```

---

**This is a fixture.** It lives in `samples/perl/` so the application has a
Perl project to open, not just a Perl file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
