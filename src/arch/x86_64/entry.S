    .macro PUSH_SCRATCH
        push rax
        push rcx
        push rdx
        push rsi
        push rdi
        push r8
        push r9
        push r10
        push r11
    .endm

    .macro POP_SCRATCH
        pop r11
        pop r10
        pop r9
        pop r8
        pop rdi
        pop rsi
        pop rdx
        pop rcx
        pop rax
    .endm

    .macro ISR_NOERR name, vector
    .global \name
    .type \name,@function
\name:
        push 0
        push \vector
        jmp __mirage_interrupt_entry
    .endm

    .macro ISR_ERR name, vector
    .global \name
    .type \name,@function
\name:
        push \vector
        jmp __mirage_interrupt_entry
    .endm

    ISR_NOERR __mirage_isr_divide_error, 0
    ISR_NOERR __mirage_isr_debug, 1
    ISR_NOERR __mirage_isr_non_maskable, 2
    ISR_NOERR __mirage_isr_breakpoint, 3
    ISR_NOERR __mirage_isr_overflow, 4
    ISR_NOERR __mirage_isr_bound_range, 5
    ISR_NOERR __mirage_isr_invalid_opcode, 6
    ISR_NOERR __mirage_isr_device_not_available, 7
    ISR_ERR   __mirage_isr_double_fault, 8
    ISR_ERR   __mirage_isr_invalid_tss, 10
    ISR_ERR   __mirage_isr_segment_not_present, 11
    ISR_ERR   __mirage_isr_stack_segment_fault, 12
    ISR_ERR   __mirage_isr_general_protection, 13
    ISR_ERR   __mirage_isr_page_fault, 14
    ISR_NOERR __mirage_isr_x87, 16
    ISR_ERR   __mirage_isr_alignment_check, 17
    ISR_NOERR __mirage_isr_machine_check, 18
    ISR_NOERR __mirage_isr_simd, 19
    ISR_NOERR __mirage_isr_virtualization, 20
    ISR_NOERR __mirage_isr_timer, 32
    ISR_NOERR __mirage_isr_syscall_trap, 128

    .section .bss
    .align 8
__mirage_scheduler_rsp:
    .quad 0

    .section .text
    .global __mirage_interrupt_entry
    .type __mirage_interrupt_entry,@function
__mirage_interrupt_entry:
        cld
        sub rsp, 216

        /* CpuContext register area: keep in sync with src/kernel/thread.rs. */
        mov [rsp + 0], rax
        mov [rsp + 8], rbx
        mov [rsp + 16], rcx
        mov [rsp + 24], rdx
        mov [rsp + 32], rsi
        mov [rsp + 40], rdi
        mov [rsp + 48], rbp
        mov [rsp + 56], r8
        mov [rsp + 64], r9
        mov [rsp + 72], r10
        mov [rsp + 80], r11
        mov [rsp + 88], r12
        mov [rsp + 96], r13
        mov [rsp + 104], r14
        mov [rsp + 112], r15

        /* Hardware/software interrupt frame above the CpuContext frame. */
        mov rax, [rsp + 216 + 16]
        mov [rsp + 120], rax
        mov rax, [rsp + 216 + 24]
        mov [rsp + 128], rax
        mov [rsp + 208], rax
        and qword ptr [rsp + 208], 3
        mov rax, [rsp + 216 + 32]
        mov [rsp + 136], rax
        mov rax, [rsp + 216 + 40]
        mov [rsp + 144], rax
        mov rax, [rsp + 216 + 48]
        mov [rsp + 152], rax
        mov rax, [rsp + 216 + 0]
        mov [rsp + 192], rax
        mov rax, [rsp + 216 + 8]
        mov [rsp + 200], rax

        mov rax, fs
        mov [rsp + 160], rax
        mov rax, gs
        mov [rsp + 168], rax

        mov ecx, 0xc0000100
        rdmsr
        shl rdx, 32
        or rax, rdx
        mov [rsp + 176], rax

        mov rax, [rsp + 192]
        cmp rax, 128
        jne 1f
        mov ecx, 0xc0000102
        jmp 2f
1:
        mov ecx, 0xc0000101
2:
        rdmsr
        shl rdx, 32
        or rax, rdx
        mov [rsp + 184], rax

        mov rdi, rsp
        mov rsi, [rip + __mirage_current_core]
        mov rdx, [rip + __mirage_current_thread]
        call __mirage_arch_save_trap_frame

        mov rax, [rsp + 192]
        cmp rax, 32
        je 3f
        cmp rax, 128
        jne 4f
3:
        mov rax, [rip + __mirage_scheduler_rsp]
        test rax, rax
        je 4f
        mov qword ptr [rip + __mirage_scheduler_rsp], 0
        mov rsp, rax
        ret

4:
        mov rax, [rsp + 0]
        mov rbx, [rsp + 8]
        mov rcx, [rsp + 16]
        mov rdx, [rsp + 24]
        mov rsi, [rsp + 32]
        mov rdi, [rsp + 40]
        mov rbp, [rsp + 48]
        mov r8,  [rsp + 56]
        mov r9,  [rsp + 64]
        mov r10, [rsp + 72]
        mov r11, [rsp + 80]
        mov r12, [rsp + 88]
        mov r13, [rsp + 96]
        mov r14, [rsp + 104]
        mov r15, [rsp + 112]
        add rsp, 232
        iretq

    .global __mirage_syscall_entry
    .type __mirage_syscall_entry,@function
__mirage_syscall_entry:
        swapgs
        push 0x23
        push rsp
        push r11
        push 0x1b
        push rcx
        push 0
        push 128
        jmp __mirage_interrupt_entry

    .global __mirage_context_save
    .type __mirage_context_save,@function
__mirage_context_save:
        mov [rdi + 0], rax
        mov [rdi + 8], rbx
        mov [rdi + 16], rcx
        mov [rdi + 24], rdx
        mov [rdi + 32], rsi
        mov [rdi + 40], rdi
        mov [rdi + 48], rbp
        mov [rdi + 56], r8
        mov [rdi + 64], r9
        mov [rdi + 72], r10
        mov [rdi + 80], r11
        mov [rdi + 88], r12
        mov [rdi + 96], r13
        mov [rdi + 104], r14
        mov [rdi + 112], r15
        lea rax, [rip + 1f]
        mov [rdi + 120], rax
        pushfq
        pop rax
        mov [rdi + 136], rax
        mov [rdi + 144], rsp
        mov rax, cs
        mov [rdi + 128], rax
        mov rax, ss
        mov [rdi + 152], rax
        mov rax, fs
        mov [rdi + 160], rax
        mov rax, gs
        mov [rdi + 168], rax
        xor rax, rax
        mov [rdi + 176], rax
        mov [rdi + 184], rax
        mov [rdi + 192], rax
        mov [rdi + 200], rax
        mov rax, [rdi + 128]
        and rax, 3
        mov [rdi + 208], rax
1:
        ret

    .global __mirage_context_restore
    .type __mirage_context_restore,@function
__mirage_context_restore:
        lea rax, [rip + __mirage_scheduler_rsp]
        mov [rax], rsp
        mov rbx, [rdi + 8]
        mov rcx, [rdi + 16]
        mov rdx, [rdi + 24]
        mov rsi, [rdi + 32]
        mov rbp, [rdi + 48]
        mov r8,  [rdi + 56]
        mov r9,  [rdi + 64]
        mov r10, [rdi + 72]
        mov r11, [rdi + 80]
        mov r12, [rdi + 88]
        mov r13, [rdi + 96]
        mov r14, [rdi + 104]
        mov r15, [rdi + 112]
        mov rax, [rdi + 152]
        push rax
        mov rax, [rdi + 144]
        push rax
        mov rax, [rdi + 136]
        push rax
        mov rax, [rdi + 128]
        push rax
        mov rax, [rdi + 120]
        push rax
        mov rax, [rdi + 0]
        mov rdi, [rdi + 40]
        iretq

    .global __mirage_return_to_user_mode
    .type __mirage_return_to_user_mode,@function
__mirage_return_to_user_mode:
        jmp __mirage_context_restore

    .global __mirage_return_to_kernel_mode
    .type __mirage_return_to_kernel_mode,@function
__mirage_return_to_kernel_mode:
        jmp __mirage_context_restore
