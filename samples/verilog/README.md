# counter

A small SystemVerilog design: a parameterised counter, the block that
instantiates it, and a testbench.

    verilator --lint-only rtl/counter.sv
    iverilog -g2012 -o counter_tb rtl/counter.sv tb/counter_tb.sv

The `always @(posedge clk)` block in `top` is written the old way rather
than as `always_ff`. It is left that way so the file pane has something
to warn about.
