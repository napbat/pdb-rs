; A MASM program for ml /Zi (x86). Like x64.asm, and also NEAR and FAR TYPEDEF pointers
; (LF_POINTER kinds near32 and far32), a PROC with parameters, USES and LOCAL data.
.686
.model flat, stdcall
option casemap:none
POINT2 STRUCT
    x SDWORD ?
    y SDWORD ?
POINT2 ENDS
MIXED UNION
    asInt DWORD ?
    asReal REAL4 ?
MIXED ENDS
FLAGS RECORD ready:1, mode:3, level:4
PBYTE TYPEDEF PTR BYTE
NPPOINT TYPEDEF NEAR PTR POINT2
FPPOINT TYPEDEF FAR PTR POINT2
PPDWORD TYPEDEF PTR PTR DWORD
COUNT_T TYPEDEF DWORD
ExitProcess PROTO :DWORD
.data
b8 BYTE 1
sb8 SBYTE -1
sw16 SWORD -2
f48 FWORD 4
q64 QWORD 5
sq64 SQWORD -5
t80 TBYTE 6
vr10 REAL10 3.5
o128 OWORD ?
pt POINT2 <1, 2>
mx MIXED <>
fl FLAGS <1, 2, 3>
pb PBYTE OFFSET b8
np NPPOINT OFFSET pt
fp FPPOINT ?
ppd PPDWORD ?
cnt COUNT_T 11
.code
helper PROC uses ebx esi, count:DWORD, scale:SDWORD
    LOCAL tmp:DWORD, buf[4]:BYTE, ext:REAL10, ptl:POINT2
    mov eax, count
    mov tmp, eax
    mov ptl.x, eax
    imul eax, scale
here:
    ret
helper ENDP
start PROC
    invoke helper, 3, 4
    invoke ExitProcess, eax
start ENDP
END start
