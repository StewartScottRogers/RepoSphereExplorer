# surveytools

Clean, describe and model a small survey: generate a plausible one, drop
what cannot be used, summarise each column, and fit a linear model whose
coefficients come back as a data frame rather than a printout.

## Using it

```r
survey <- clean_survey(make_survey(n = 240))

by_team(survey)
tidy_coefficients(fit_model(survey), digits = 3)

report(survey)
```

## Notes

- `make_survey` respects `set.seed`, so an analysis nobody can reproduce is
  not the default.
- `tidy_coefficients` returns a data frame. A model summary printed to the
  console is a model summary nobody can join to anything.
- `clean_survey` never invents rows. Imputing quietly is how a report ends
  up describing data that was never collected.

## Developing

```r
devtools::test()
devtools::check()
```

---

**This is a fixture.** It lives in `samples/r/` so the application has a
R project to open, not just a R file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
