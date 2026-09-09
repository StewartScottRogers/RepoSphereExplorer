import 'package:test/test.dart';
import 'package:weather/weather.dart';

void main() {
  group('Reading', () {
    test('converts to fahrenheit without going through a string', () {
      final reading = Reading(station: 'LHR', celsius: 100, conditions: Conditions.clear);

      expect(reading.fahrenheit, closeTo(212, 0.001));
    });

    test('describes itself for a log line', () {
      final reading = Reading(station: 'LHR', celsius: 12.34, conditions: Conditions.clear);

      expect(reading.summary, contains('LHR'));
      expect(reading.summary, contains('12.3'));
    });

    test('round-trips through JSON', () {
      final reading = Reading(station: 'EDI', celsius: -3.5, conditions: Conditions.snow);

      final restored = Reading.fromJson(reading.toJson());

      expect(restored.station, reading.station);
      expect(restored.celsius, reading.celsius);
      expect(restored.conditions, reading.conditions);
    });
  });

  group('RetryingSource', () {
    test('returns what the underlying source returns', () async {
      final source = RetryingSource(FakeSource(), attempts: 3);

      final reading = await source.current('LHR');

      expect(reading.station, 'LHR');
    });

    test('gives up rather than retrying for ever', () async {
      final source = RetryingSource(_AlwaysFails(), attempts: 2);

      await expectLater(() => source.current('LHR'), throwsA(isA<WeatherException>()));
    });
  });
}

/// A source that never succeeds, so the retry limit is visible.
class _AlwaysFails implements WeatherSource {
  @override
  Future<Reading> current(String station) async {
    throw WeatherException('nothing is listening');
  }
}
