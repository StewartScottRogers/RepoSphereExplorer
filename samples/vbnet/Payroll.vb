Imports System
Imports System.Collections.Generic
Imports System.Globalization
Imports System.Linq

Namespace Payroll

    ''' <summary>How an employee is paid.</summary>
    Public Enum PayBasis
        Hourly
        Salaried
        Contract
    End Enum

    Public Interface IPayable
        ReadOnly Property Reference As String
        Function GrossFor(period As PayPeriod) As Decimal
    End Interface

    Public Structure PayPeriod
        Public ReadOnly Starts As Date
        Public ReadOnly Ends As Date

        Public Sub New(starts As Date, ends As Date)
            If ends < starts Then
                Throw New ArgumentException("a pay period cannot end before it starts", NameOf(ends))
            End If
            Me.Starts = starts
            Me.Ends = ends
        End Sub

        Public ReadOnly Property Days As Integer
            Get
                Return (Ends - Starts).Days + 1
            End Get
        End Property

        Public Overrides Function ToString() As String
            Return $"{Starts:yyyy-MM-dd} to {Ends:yyyy-MM-dd}"
        End Function
    End Structure

    Public Class Employee
        Implements IPayable

        Public Property Reference As String Implements IPayable.Reference
        Public Property FullName As String
        Public Property Basis As PayBasis
        Public Property AnnualSalary As Decimal
        Public Property HourlyRate As Decimal
        Public Property HoursWorked As Decimal
        Public Property TaxCode As String = "1257L"

        Public Sub New(reference As String, fullName As String, basis As PayBasis)
            Me.Reference = reference
            Me.FullName = fullName
            Me.Basis = basis
        End Sub

        Public Function GrossFor(period As PayPeriod) As Decimal Implements IPayable.GrossFor
            Select Case Basis
                Case PayBasis.Salaried
                    Return Math.Round(AnnualSalary / 365D * period.Days, 2)
                Case PayBasis.Hourly
                    Return Math.Round(HourlyRate * HoursWorked, 2)
                Case PayBasis.Contract
                    Return Math.Round(HourlyRate * HoursWorked * 1.05D, 2)
                Case Else
                    Return 0D
            End Select
        End Function

        Public Overrides Function ToString() As String
            Return $"{Reference} {FullName} ({Basis})"
        End Function
    End Class

    Public Module TaxRules

        Private ReadOnly PersonalAllowance As Decimal = 12570D
        Private ReadOnly BasicRateLimit As Decimal = 50270D

        Public Function AnnualTax(gross As Decimal) As Decimal
            If gross <= PersonalAllowance Then
                Return 0D
            End If

            Dim basic = Math.Min(gross, BasicRateLimit) - PersonalAllowance
            Dim higher = Math.Max(0D, gross - BasicRateLimit)
            Return Math.Round(basic * 0.2D + higher * 0.4D, 2)
        End Function

        Public Function NationalInsurance(gross As Decimal) As Decimal
            Dim threshold = 12570D
            If gross <= threshold Then
                Return 0D
            End If
            Return Math.Round((gross - threshold) * 0.08D, 2)
        End Function

        Public Function NetFor(employee As Employee, period As PayPeriod) As Decimal
            Dim gross = employee.GrossFor(period)
            Dim annualised = gross * (365D / period.Days)
            Dim tax = AnnualTax(annualised) / (365D / period.Days)
            Dim ni = NationalInsurance(annualised) / (365D / period.Days)
            Return Math.Round(gross - tax - ni, 2)
        End Function

    End Module

    Public Class PayRun

        Private ReadOnly _employees As New List(Of Employee)()

        Public Sub Add(employee As Employee)
            _employees.Add(employee)
        End Sub

        Public Function Lines(period As PayPeriod) As IEnumerable(Of String)
            Return _employees.
                OrderBy(Function(e) e.Reference).
                Select(Function(e)
                           Dim gross = e.GrossFor(period)
                           Dim net = TaxRules.NetFor(e, period)
                           Return String.Format(CultureInfo.InvariantCulture,
                                                "{0,-8} {1,-20} {2,10:N2} {3,10:N2}",
                                                e.Reference, e.FullName, gross, net)
                       End Function)
        End Function

        Public Function TotalGross(period As PayPeriod) As Decimal
            Return _employees.Sum(Function(e) e.GrossFor(period))
        End Function

        Public Shared Sub Main()
            Dim period = New PayPeriod(New Date(2026, 9, 1), New Date(2026, 9, 30))
            Dim run = New PayRun()

            run.Add(New Employee("E-001", "Ada Lovelace", PayBasis.Salaried) With {.AnnualSalary = 68000D})
            run.Add(New Employee("E-002", "Alan Turing", PayBasis.Hourly) With {
                .HourlyRate = 42.5D, .HoursWorked = 148D})

            Console.WriteLine(period.ToString())
            For Each line In run.Lines(period)
                Console.WriteLine(line)
            Next
            Console.WriteLine($"total gross {run.TotalGross(period):N2}")
        End Sub

    End Class

End Namespace
