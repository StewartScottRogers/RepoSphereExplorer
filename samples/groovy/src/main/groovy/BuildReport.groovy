#!/usr/bin/env groovy
/*
 * Summarises a set of build results: parses the JSON a CI job leaves
 * behind, groups it by project, and prints a table plus a non-zero exit
 * status when anything failed.
 */

import groovy.json.JsonSlurper
import groovy.transform.CompileStatic
import groovy.transform.ToString
import java.time.Duration
import java.time.Instant

enum Outcome {
    PASSED,
    FAILED,
    SKIPPED

    boolean isProblem() {
        this == FAILED
    }
}

@ToString(includeNames = true)
class BuildResult {
    String project
    String job
    Outcome outcome
    Duration took
    String commit

    static BuildResult fromMap(Map raw) {
        new BuildResult(
            project: raw.project as String,
            job: raw.job as String,
            outcome: Outcome.valueOf((raw.outcome as String).toUpperCase()),
            took: Duration.ofMillis((raw.durationMs ?: 0) as long),
            commit: (raw.commit as String)?.take(8)
        )
    }

    String getSummary() {
        "${project}/${job} ${outcome} in ${took.toSeconds()}s"
    }
}

@CompileStatic
class ReportBuilder {
    private final List<BuildResult> results = []

    ReportBuilder add(BuildResult result) {
        results << result
        this
    }

    Map<String, List<BuildResult>> byProject() {
        results.groupBy { BuildResult result -> result.project }
    }

    List<BuildResult> failures() {
        results.findAll { BuildResult result -> result.outcome.problem }
    }

    Duration total() {
        results.inject(Duration.ZERO) { Duration acc, BuildResult result -> acc.plus(result.took) }
    }

    int size() {
        results.size()
    }
}

class ReportPrinter {
    private static final String ROW = '%-14s %-16s %-8s %6ss%n'

    void print(ReportBuilder report) {
        printf('%-14s %-16s %-8s %7s%n', 'PROJECT', 'JOB', 'OUTCOME', 'TIME')
        report.byProject().each { String project, List<BuildResult> results ->
            results.sort { it.job }.each { BuildResult result ->
                printf(ROW, project, result.job, result.outcome, result.took.toSeconds())
            }
        }
        println "total ${report.total().toSeconds()}s across ${report.size()} jobs"
    }
}

static List<BuildResult> parse(String json) {
    new JsonSlurper().parseText(json).collect { BuildResult.fromMap(it as Map) }
}

def payload = '''
[
  {"project": "explorer", "job": "clippy", "outcome": "passed",  "durationMs": 41000, "commit": "1ee09965"},
  {"project": "explorer", "job": "tests",  "outcome": "failed",  "durationMs": 92000, "commit": "1ee09965"},
  {"project": "updater",  "job": "build",  "outcome": "passed",  "durationMs": 15500, "commit": "d3b44a81"},
  {"project": "updater",  "job": "audit",  "outcome": "skipped", "durationMs": 0,     "commit": "d3b44a81"}
]
'''

def report = new ReportBuilder()
parse(payload).each { report.add(it) }

new ReportPrinter().print(report)

def failures = report.failures()
if (failures) {
    System.err.println("failed: ${failures*.summary.join(', ')}")
    System.exit(1)
}
