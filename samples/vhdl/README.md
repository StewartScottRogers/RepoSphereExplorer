# counter

A small VHDL design: a parameterised counter, a structural block that
instantiates it, and a testbench.

    ghdl -a rtl/counter.vhd tb/counter_tb.vhd
    ghdl -r counter_tb --stop-time=500ns

The `latched` process in `top` has no sensitivity list and no `wait`, so
it runs once when simulation starts and never again. It is left that way
so the file pane has something to warn about.
