# Payroll

Pay periods, gross pay and deductions, with money as `Decimal` throughout.

## Using it

```vbnet
Dim period As New PayPeriod(startsOn, endsOn)
Dim gross As Decimal = employee.GrossFor(period)
```

## Notes

- `Option Strict On` in both the project file and `Directory.Build.props`,
  so a project added later cannot opt out. Late binding in a payroll
  calculation is how a rounding bug reaches somebody's payslip.
- Money is `Decimal`, never `Double`. Binary floating point cannot
  represent a penny, and payroll is the one place that always shows.
- `IPayable` is an interface rather than a base class, so an hourly worker,
  a salaried one and a contractor share a contract without sharing an
  inheritance chain.

## Building

```bash
dotnet build
```

---

**This is a fixture.** It lives in `samples/vbnet/` so the application has a
Visual Basic .NET project to open, not just a Visual Basic .NET file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
