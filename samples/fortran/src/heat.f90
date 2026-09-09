! Solves the 1-D heat equation with an explicit finite-difference scheme,
! reports the residual each step, and writes the final profile.
!
! Build: gfortran -O2 -o heat heat.f90

module heat_kernel
    implicit none
    private

    integer, parameter, public :: dp = selected_real_kind(15, 307)

    public :: initial_profile, step, residual, write_profile

contains

    subroutine initial_profile(field, left, right)
        real(dp), intent(out) :: field(:)
        real(dp), intent(in)  :: left, right
        integer :: i, n

        n = size(field)
        do i = 1, n
            field(i) = left + (right - left) * real(i - 1, dp) / real(n - 1, dp)
        end do
        field(n / 3 : 2 * n / 3) = field(n / 3 : 2 * n / 3) + 25.0_dp
    end subroutine initial_profile

    subroutine step(field, alpha)
        real(dp), intent(inout) :: field(:)
        real(dp), intent(in)    :: alpha
        real(dp), allocatable   :: next(:)
        integer :: i, n

        n = size(field)
        allocate(next(n))
        next = field

        do concurrent (i = 2 : n - 1)
            next(i) = field(i) + alpha * (field(i - 1) - 2.0_dp * field(i) + field(i + 1))
        end do

        field = next
        deallocate(next)
    end subroutine step

    function residual(field, previous) result(value)
        real(dp), intent(in) :: field(:), previous(:)
        real(dp) :: value

        value = sqrt(sum((field - previous) ** 2) / real(size(field), dp))
    end function residual

    subroutine write_profile(unit, field)
        integer, intent(in)  :: unit
        real(dp), intent(in) :: field(:)
        integer :: i

        do i = 1, size(field)
            write(unit, '(I5, 1X, F12.6)') i, field(i)
        end do
    end subroutine write_profile

end module heat_kernel

program heat
    use heat_kernel
    implicit none

    integer, parameter :: cells = 64
    integer, parameter :: max_steps = 500
    real(dp), parameter :: alpha = 0.20_dp
    real(dp), parameter :: tolerance = 1.0e-8_dp

    real(dp) :: field(cells), previous(cells)
    real(dp) :: change
    integer  :: iteration, unit, stat

    call initial_profile(field, 20.0_dp, 80.0_dp)

    do iteration = 1, max_steps
        previous = field
        call step(field, alpha)
        change = residual(field, previous)

        if (mod(iteration, 100) == 0) then
            write(*, '(A, I4, A, ES12.4)') 'step ', iteration, '  residual ', change
        end if

        if (change < tolerance) then
            write(*, '(A, I4)') 'converged at step ', iteration
            exit
        end if
    end do

    open(newunit=unit, file='heat_profile.txt', status='replace', action='write', iostat=stat)
    if (stat /= 0) then
        write(*, '(A)') 'could not open heat_profile.txt'
        stop 1
    end if

    call write_profile(unit, field)
    close(unit)

    write(*, '(A, F8.3, A, F8.3)') 'ends at ', field(1), ' .. ', field(cells)
end program heat
