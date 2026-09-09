# Analyses a small survey: cleans the raw responses, fits a linear model
# against satisfaction, and prints a tidy summary. Base R only, so it runs
# anywhere without installing anything.

set.seed(20260908)

#' Build a synthetic survey data frame.
#'
#' @param n Number of respondents.
#' @return A data frame with one row per respondent.
make_survey <- function(n = 240) {
  tenure <- pmax(0, round(rnorm(n, mean = 3.5, sd = 2.2), 1))
  team <- factor(sample(c("platform", "product", "support"), n, replace = TRUE))
  workload <- sample(1:5, n, replace = TRUE, prob = c(0.1, 0.2, 0.35, 0.25, 0.1))

  satisfaction <- 6.5 +
    0.35 * tenure -
    0.60 * workload +
    ifelse(team == "support", -0.8, 0) +
    rnorm(n, sd = 0.9)

  data.frame(
    id = seq_len(n),
    tenure_years = tenure,
    team = team,
    workload = workload,
    satisfaction = round(pmin(10, pmax(0, satisfaction)), 2),
    stringsAsFactors = FALSE
  )
}

#' Drop impossible rows and report how many went.
clean_survey <- function(survey) {
  before <- nrow(survey)
  kept <- subset(survey, !is.na(satisfaction) & tenure_years >= 0 & workload %in% 1:5)
  attr(kept, "dropped") <- before - nrow(kept)
  kept
}

#' Summary statistics for one numeric column.
describe_column <- function(values, name) {
  quantiles <- quantile(values, probs = c(0.25, 0.5, 0.75), names = FALSE)
  data.frame(
    column = name,
    n = length(values),
    mean = mean(values),
    sd = sd(values),
    q25 = quantiles[1],
    median = quantiles[2],
    q75 = quantiles[3]
  )
}

#' Per-team means, sorted worst first.
by_team <- function(survey) {
  aggregated <- aggregate(satisfaction ~ team, data = survey, FUN = mean)
  aggregated$n <- as.vector(table(survey$team)[as.character(aggregated$team)])
  aggregated[order(aggregated$satisfaction), ]
}

#' Fit satisfaction against tenure, workload and team.
fit_model <- function(survey) {
  lm(satisfaction ~ tenure_years + workload + team, data = survey)
}

#' Turn a fitted model into a small coefficient table.
tidy_coefficients <- function(model, digits = 3) {
  coefficients <- summary(model)$coefficients
  out <- data.frame(
    term = rownames(coefficients),
    estimate = round(coefficients[, "Estimate"], digits),
    std_error = round(coefficients[, "Std. Error"], digits),
    p_value = signif(coefficients[, "Pr(>|t|)"], digits),
    row.names = NULL
  )
  out[order(out$p_value), ]
}

#' Print everything a reader of this script would want.
report <- function(survey = make_survey()) {
  cleaned <- clean_survey(survey)
  message(sprintf("kept %d of %d responses", nrow(cleaned), nrow(survey)))

  numeric_columns <- c("tenure_years", "workload", "satisfaction")
  described <- do.call(rbind, lapply(numeric_columns, function(name) {
    describe_column(cleaned[[name]], name)
  }))
  print(described, row.names = FALSE, digits = 3)

  cat("\nby team:\n")
  print(by_team(cleaned), row.names = FALSE, digits = 3)

  model <- fit_model(cleaned)
  cat("\ncoefficients:\n")
  print(tidy_coefficients(model), row.names = FALSE)

  cat(sprintf("\nadjusted R-squared: %.3f\n", summary(model)$adj.r.squared))
  invisible(model)
}

if (identical(environment(), globalenv()) && !interactive()) {
  report()
}
