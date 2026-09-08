/*
 * x86-64 System V assembly: a Fletcher-16 checksum over a byte buffer,
 * plus a small entry point that runs it and exits with the low byte.
 *
 * Assemble and link with:
 *   as --64 -o checksum.o checksum.s && ld -o checksum checksum.o
 */

    .intel_syntax noprefix

    .global _start
    .global fletcher16
    .global buffer_length

    .section .rodata
message:
    .ascii "RepoSphereExplorer sample buffer\n"
message_end:
    .set message_length, message_end - message

banner:
    .asciz "checksum: "

    .section .data
    .align 8
buffer_length:
    .quad message_length
last_result:
    .word 0

    .section .bss
    .align 16
scratch:
    .space 64

    .section .text

/*
 * fletcher16(rdi = buffer, rsi = length) -> ax
 * Clobbers rax, rcx, rdx, r8, r9.
 */
fletcher16:
    xor     r8, r8              /* sum1 */
    xor     r9, r9              /* sum2 */
    test    rsi, rsi
    jz      .Lfletcher_done

.Lfletcher_loop:
    movzx   rax, byte ptr [rdi]
    add     r8, rax
    mov     rax, r8
    mov     rcx, 255
    xor     rdx, rdx
    div     rcx
    mov     r8, rdx             /* sum1 %= 255 */

    add     r9, r8
    mov     rax, r9
    xor     rdx, rdx
    div     rcx
    mov     r9, rdx             /* sum2 %= 255 */

    inc     rdi
    dec     rsi
    jnz     .Lfletcher_loop

.Lfletcher_done:
    mov     rax, r9
    shl     rax, 8
    or      rax, r8
    mov     word ptr [rip + last_result], ax
    ret

/*
 * write_message(): writes the sample buffer to stdout.
 */
write_message:
    mov     rax, 1              /* sys_write */
    mov     rdi, 1              /* stdout */
    lea     rsi, [rip + message]
    mov     rdx, message_length
    syscall
    ret

_start:
    call    write_message

    lea     rdi, [rip + message]
    mov     rsi, [rip + buffer_length]
    call    fletcher16

    movzx   rdi, al             /* exit status is the low byte */
    mov     rax, 60             /* sys_exit */
    syscall

.Lunreachable:
    hlt
