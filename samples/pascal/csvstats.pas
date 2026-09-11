{ Summary statistics for a column of a comma-separated file. }
unit CsvStats;

{$MODE OBJFPC}{$H+}

interface

uses
  SysUtils, Classes;

type
  { One reading, as it comes out of the file. }
  TSample = record
    Name: string;
    Value: Double;
  end;

  ESampleError = class(Exception);

  { A named column of numbers. }
  TColumn = class(TObject)
  private
    FName: string;
    FValues: array of Double;
    function GetCount: Integer;
  public
    constructor Create(const AName: string);
    destructor Destroy; override;
    procedure Add(const Value: Double);
    function Mean: Double;
    function Deviation: Double;
    property Name: string read FName;
    property Count: Integer read GetCount;
  end;

function Largest(const Values: array of Double): Double;

{ Promised here, and never written below: the unit will not link until
  somebody writes it. It is left that way on purpose. }
procedure WriteReport(const Path: string);

implementation

uses
  Math;

constructor TColumn.Create(const AName: string);
begin
  inherited Create;
  FName := AName;
  SetLength(FValues, 0);
end;

destructor TColumn.Destroy;
begin
  SetLength(FValues, 0);
  inherited Destroy;
end;

function TColumn.GetCount: Integer;
begin
  Result := Length(FValues);
end;

procedure TColumn.Add(const Value: Double);
begin
  SetLength(FValues, Length(FValues) + 1);
  FValues[High(FValues)] := Value;
end;

function TColumn.Mean: Double;
var
  Value: Double;
  Total: Double;
begin
  if Length(FValues) = 0 then
    raise ESampleError.CreateFmt('column %s is empty', [FName]);
  Total := 0;
  for Value in FValues do
    Total := Total + Value;
  Result := Total / Length(FValues);
end;

function TColumn.Deviation: Double;
var
  Value: Double;
  Average: Double;
  Total: Double;
begin
  Average := Mean;
  Total := 0;
  for Value in FValues do
    Total := Total + Sqr(Value - Average);
  Result := Sqrt(Total / Length(FValues));
end;

function Largest(const Values: array of Double): Double;
var
  Value: Double;
begin
  Result := Values[Low(Values)];
  for Value in Values do
    if Value > Result then
      Result := Value;
end;

end.
