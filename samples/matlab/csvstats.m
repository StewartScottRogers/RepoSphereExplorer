function [mean_value, deviation, count] = csvstats(values, options)
%CSVSTATS Summary statistics for a column of readings.
%   [M, D, N] = CSVSTATS(V) returns the mean, the population standard
%   deviation and the number of readings in V.
%
%   [...] = CSVSTATS(V, Trim=true) drops the largest and smallest
%   reading before doing the arithmetic.

    arguments
        values (1,:) double
        options.Trim (1,1) logical = false
    end

    if options.Trim && numel(values) > 2
        values = trim_ends(values);
    end

    count = numel(values);
    mean_value = sum(values) / count;
    deviation = spread(values, mean_value);

    function s = spread(v, m)
        % Nested: it can see `count` above without being handed it.
        s = sqrt(sum((v - m) .^ 2) / count);
    end
end

function trimmed = trim_ends(values)
% A local function: after the first one has ended, not inside it.
% It takes an input and checks nothing about it.
    sorted = sort(values);
    trimmed = sorted(2:end-1);
end

function clean = drop_missing(raw)
% Another local function, also unchecked.
    clean = raw(~isnan(raw));
end
