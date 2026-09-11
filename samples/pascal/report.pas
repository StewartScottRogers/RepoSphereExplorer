program report;

{$MODE OBJFPC}{$H+}

uses
  SysUtils, CsvStats;

var
  Column: TColumn;

begin
  Column := TColumn.Create('example');
  try
    Column.Add(1);
    Column.Add(2);
    Column.Add(3);
    WriteLn(Format('%-20s %10.3f %10.3f', [Column.Name, Column.Mean, Column.Deviation]));
  except
    on E: ESampleError do
      WriteLn(ErrOutput, 'nothing to report: ', E.Message);
  end;
  Column.Free;
end.
