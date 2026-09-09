import java.time.Duration
import spock.lang.Specification
import spock.lang.Subject

class BuildReportSpec extends Specification {

    @Subject
    ReportBuilder report = new ReportBuilder()

    private static BuildResult result(String project, String task, boolean ok, int seconds) {
        new BuildResult(
            project: project,
            task: task,
            status: ok ? 'SUCCESS' : 'FAILED',
            duration: Duration.ofSeconds(seconds)
        )
    }

    def 'an empty report has nothing in it'() {
        expect:
        report.size() == 0
        report.failures().isEmpty()
    }

    def 'every added result is counted'() {
        when:
        report.add(result('api', 'compile', true, 4))
        report.add(result('api', 'test', true, 9))

        then:
        report.size() == 2
    }

    def 'results group by the project they came from'() {
        given:
        report.add(result('api', 'compile', true, 4))
        report.add(result('web', 'compile', true, 6))

        expect:
        report.byProject().keySet() == ['api', 'web'] as Set
    }

    def 'only the failures come back as failures'() {
        given:
        report.add(result('api', 'compile', true, 4))
        report.add(result('api', 'test', false, 12))

        when:
        def failures = report.failures()

        then:
        failures.size() == 1
        failures.first().task == 'test'
    }

    def 'the total is the sum of every duration, not just the failures'() {
        given:
        report.add(result('api', 'compile', true, 4))
        report.add(result('api', 'test', false, 12))

        expect:
        report.total() == Duration.ofSeconds(16)
    }
}
