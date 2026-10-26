; A MASM program for ml64 /Zi. MASM writes its own CodeView records: STRUCT, UNION and
; RECORD types, TYPEDEF pointers (LF_POINTER with 6 zero bytes after the pointer data) and data
; of each MASM type. FWORD gives T_REAL48, SQWORD T_INT8, TBYTE and REAL10 T_REAL80, OWORD
; and XMMWORD T_UINT16.
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
PPOINT TYPEDEF PTR POINT2
COUNT_T TYPEDEF DWORD
.data
b8 BYTE 1
sb8 SBYTE -1
w16 WORD 2
sw16 SWORD -2
d32 DWORD 3
sd32 SDWORD -3
f48 FWORD 4
q64 QWORD 5
sq64 SQWORD -5
t80 TBYTE 6
vr4 REAL4 1.5
vr8 REAL8 2.5
vr10 REAL10 3.5
o128 OWORD ?
x128 XMMWORD ?
y256 YMMWORD ?
mm64 MMWORD ?
pt POINT2 <1, 2>
mx MIXED <>
fl FLAGS <1, 2, 3>
pb PBYTE OFFSET b8
pp PPOINT OFFSET pt
cnt COUNT_T 11
arr DWORD 4 DUP (0)
.code
ExitProcess PROTO
helper PROC FRAME
    push rbp
    .pushreg rbp
    mov rbp, rsp
    .setframe rbp, 0
    .endprolog
    mov eax, d32
    add eax, cnt
local_label:
    pop rbp
    ret
helper ENDP
mainCRTStartup PROC FRAME
    sub rsp, 40
    .allocstack 40
    .endprolog
    call helper
    mov ecx, eax
    call ExitProcess
mainCRTStartup ENDP
END
