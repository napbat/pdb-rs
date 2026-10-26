
! Fortran variables of each intrinsic kind for flang. flang writes LOGICAL(2), LOGICAL(4) and
! LOGICAL(8) as T_BOOL16, T_BOOL32 and T_BOOL64, REAL(10) as T_REAL80, COMPLEX(10) as T_CPLX80
! and INTEGER(16) as T_OCT. primitives.c calls touch_kinds; it uses no Fortran runtime function.
module kinds
  implicit none
  logical(1) :: l1 = .true.
  logical(2) :: l2 = .true.
  logical(4) :: l4 = .false.
  logical(8) :: l8 = .true.
  integer(1) :: i1 = 1
  integer(2) :: i2 = 2
  integer(8) :: i8 = 8
  integer(16) :: i16 = 16
  real(2) :: r2 = 1.0
  real(10) :: r10 = 2.0
  complex(4) :: c4 = (1.0, 2.0)
  complex(8) :: c8 = (1.0, 2.0)
  complex(10) :: c10 = (1.0, 2.0)
  type point
    integer :: x, y
    logical(4) :: on
  end type
  type(point) :: pt
contains
  function touch_kinds(seed) result(total) bind(c, name="touch_kinds")
    integer(4), value :: seed
    integer(4) :: total
    logical(4) :: flag
    logical(2) :: half_flag
    logical(8) :: wide_flag
    flag = l4 .or. (seed > 0)
    half_flag = l2
    wide_flag = l8 .and. flag
    pt%x = seed
    pt%y = i2
    pt%on = flag
    total = pt%x + pt%y
    if (half_flag .and. wide_flag) total = total + 1
  end function
end module
