classdef Column < handle
%COLUMN A named column of readings.
%   A handle class, so a Column passed to a function is the same Column.

    properties
        Name (1,1) string = ""
    end

    properties (Access = private)
        Values (1,:) double = []
    end

    methods
        function obj = Column(name)
            arguments
                name (1,1) string
            end
            obj.Name = name;
        end

        function add(obj, value)
            arguments
                obj Column
                value (1,1) double
            end
            obj.Values(end + 1) = value;
        end

        function m = mean(obj)
            m = sum(obj.Values) / numel(obj.Values);
        end
    end

    methods (Static)
        function c = fromMatrix(raw, column)
        % Static, and unchecked: a wrong column number reaches the
        % indexing before anything notices.
            c = Column("column " + column);
            for value = raw(:, column)'
                c.add(value);
            end
        end
    end
end
