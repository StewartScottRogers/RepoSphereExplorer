/// A weather client with a typed model, an abstract source, retries and a
/// stream of updates - the shapes a Dart preview should surface: classes,
/// mixins, enums, extensions, futures and streams.
library weather;

import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

enum Conditions { clear, cloudy, rain, snow, fog }

class WeatherException implements Exception {
  WeatherException(this.message, {this.cause});

  final String message;
  final Object? cause;

  @override
  String toString() => 'WeatherException: $message';
}

mixin Describable {
  String get summary;

  String describe() => 'summary: $summary';
}

class Reading with Describable {
  Reading({
    required this.station,
    required this.celsius,
    required this.conditions,
    required this.recordedAt,
  });

  factory Reading.fromJson(Map<String, dynamic> json) {
    return Reading(
      station: json['station'] as String,
      celsius: (json['celsius'] as num).toDouble(),
      conditions: Conditions.values.firstWhere(
        (candidate) => candidate.name == json['conditions'],
        orElse: () => Conditions.clear,
      ),
      recordedAt: DateTime.parse(json['recordedAt'] as String),
    );
  }

  final String station;
  final double celsius;
  final Conditions conditions;
  final DateTime recordedAt;

  double get fahrenheit => celsius * 9 / 5 + 32;

  @override
  String get summary => '$station ${celsius.toStringAsFixed(1)}C ${conditions.name}';

  Map<String, dynamic> toJson() => {
        'station': station,
        'celsius': celsius,
        'conditions': conditions.name,
        'recordedAt': recordedAt.toIso8601String(),
      };
}

abstract class WeatherSource {
  Future<Reading> current(String station);

  Stream<Reading> watch(String station, {Duration every = const Duration(minutes: 5)});
}

class FakeSource implements WeatherSource {
  FakeSource({math.Random? random}) : _random = random ?? math.Random(7);

  final math.Random _random;

  @override
  Future<Reading> current(String station) async {
    await Future<void>.delayed(const Duration(milliseconds: 10));
    return Reading(
      station: station,
      celsius: 5 + _random.nextDouble() * 20,
      conditions: Conditions.values[_random.nextInt(Conditions.values.length)],
      recordedAt: DateTime.now(),
    );
  }

  @override
  Stream<Reading> watch(String station, {Duration every = const Duration(minutes: 5)}) async* {
    while (true) {
      yield await current(station);
      await Future<void>.delayed(every);
    }
  }
}

class RetryingSource implements WeatherSource {
  RetryingSource(this.inner, {this.attempts = 3});

  final WeatherSource inner;
  final int attempts;

  @override
  Future<Reading> current(String station) async {
    Object? lastError;
    for (var attempt = 1; attempt <= attempts; attempt++) {
      try {
        return await inner.current(station);
      } catch (error) {
        lastError = error;
        await Future<void>.delayed(Duration(milliseconds: 50 * attempt));
      }
    }
    throw WeatherException('gave up on $station after $attempts attempts', cause: lastError);
  }

  @override
  Stream<Reading> watch(String station, {Duration every = const Duration(minutes: 5)}) =>
      inner.watch(station, every: every);
}

extension ReadingList on List<Reading> {
  double get averageCelsius =>
      isEmpty ? 0 : map((reading) => reading.celsius).reduce((a, b) => a + b) / length;

  Reading get warmest => reduce((a, b) => a.celsius >= b.celsius ? a : b);
}

Future<void> main(List<String> arguments) async {
  final source = RetryingSource(FakeSource());
  final stations = arguments.isEmpty ? ['LHR', 'JFK', 'NRT'] : arguments;

  final readings = <Reading>[];
  for (final station in stations) {
    readings.add(await source.current(station));
  }

  for (final reading in readings) {
    print(reading.describe());
  }
  print('average ${readings.averageCelsius.toStringAsFixed(1)}C');
  print('warmest ${readings.warmest.station}');
  print(jsonEncode(readings.map((reading) => reading.toJson()).toList()));
}
