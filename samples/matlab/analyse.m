%% Load the readings
% Run this file a section at a time, or all at once.
raw = readmatrix('samples.csv');
labels = readcell('samples.csv', 'Range', '1:1');

%% Clean them up
values = raw(~isnan(raw(:, 2)), :);
percent_complete = 100 * height(values) / height(raw);
fprintf('%.1f%% of the rows survived\n', percent_complete);

%% Fit a model
% fitlm and periodogram both need a toolbox; the script fails at this
% line rather than when it is opened.
model = fitlm(values(:, 1), values(:, 2));
[power, frequency] = periodogram(values(:, 2));

%% Report
[m, d, n] = csvstats(values(:, 2)');
fprintf('n=%d mean=%.3f deviation=%.3f\n', n, m, d);
disp(model);
