! Run with `fpm test`. Each check writes what failed and stops with a
! non-zero status, because a test that prints "FAIL" and exits zero is a
! test that passes in every pipeline that matters.

program check
    use heat_kernel
    implicit none

    integer :: failures

    failures = 0

    call check_steady_state_stays_steady(failures)
    call check_heat_spreads_from_a_spike(failures)
    call check_energy_is_not_created(failures)

    if (failures > 0) then
        write (*, '(a,i0,a)') 'FAILED: ', failures, ' check(s)'
        stop 1
    end if

    write (*, '(a)') 'all checks passed'

contains

    subroutine check_steady_state_stays_steady(failures)
        integer, intent(inout) :: failures
        real(kind=8) :: field(8)

        field = 1.0d0
        call step(field, 0.1d0)

        if (any(abs(field(2:7) - 1.0d0) > 1.0d-12)) then
            write (*, '(a)') 'a uniform field should not change'
            failures = failures + 1
        end if
    end subroutine check_steady_state_stays_steady

    subroutine check_heat_spreads_from_a_spike(failures)
        integer, intent(inout) :: failures
        real(kind=8) :: field(9)

        field = 0.0d0
        field(5) = 1.0d0
        call step(field, 0.2d0)

        if (field(4) <= 0.0d0 .or. field(6) <= 0.0d0) then
            write (*, '(a)') 'heat should reach both neighbours of a spike'
            failures = failures + 1
        end if
    end subroutine check_heat_spreads_from_a_spike

    subroutine check_energy_is_not_created(failures)
        integer, intent(inout) :: failures
        real(kind=8) :: field(9), before, after

        field = 0.0d0
        field(5) = 1.0d0
        before = sum(field)
        call step(field, 0.2d0)
        after = sum(field)

        if (after > before + 1.0d-12) then
            write (*, '(a)') 'an explicit step must not create energy'
            failures = failures + 1
        end if
    end subroutine check_energy_is_not_created

end program check
