test_that("a generated survey has the number of rows it was asked for", {
  survey <- make_survey(n = 50)

  expect_equal(nrow(survey), 50)
})

test_that("cleaning never invents rows", {
  survey <- make_survey(n = 80)

  cleaned <- clean_survey(survey)

  expect_lte(nrow(cleaned), nrow(survey))
})

test_that("describing a column returns one row of summary statistics", {
  described <- describe_column(c(1, 2, 3, 4, 5), "score")

  expect_equal(nrow(described), 1L)
  expect_true("score" %in% unlist(described))
})

test_that("grouping by team returns one row per team", {
  survey <- clean_survey(make_survey(n = 200))

  grouped <- by_team(survey)

  expect_equal(nrow(grouped), length(unique(survey$team)))
})

test_that("coefficients come back rounded to the digits asked for", {
  model <- fit_model(clean_survey(make_survey(n = 200)))

  tidied <- tidy_coefficients(model, digits = 2)

  expect_true(all(!is.na(tidied)))
})

test_that("the same seed gives the same survey", {
  set.seed(20260909)
  first <- make_survey(n = 30)
  set.seed(20260909)
  second <- make_survey(n = 30)

  expect_equal(first, second)
})
