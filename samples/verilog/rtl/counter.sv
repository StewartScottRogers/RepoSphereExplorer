// A counter and the block that uses it.
//
// `stale_block` below is clocked with a bare `always @(posedge clk)`
// rather than `always_ff`. It is left that way so the file pane has
// something to warn about.

interface count_if #(
    parameter int WIDTH = 8
) (
    input logic clk
);
    logic [WIDTH-1:0] value;
    logic             overflow;

    modport source (output value, output overflow);
    modport sink   (input value, input overflow);
endinterface

module counter #(
    parameter int WIDTH = 8,
    parameter int LIMIT = 255
) (
    input  logic             clk,
    input  logic             rst_n,
    input  logic             enable,
    output logic [WIDTH-1:0] value,
    output logic             overflow
);

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            value    <= '0;
            overflow <= 1'b0;
        end else if (enable) begin
            if (value == LIMIT[WIDTH-1:0]) begin
                value    <= '0;
                overflow <= 1'b1;
            end else begin
                value    <= value + 1'b1;
                overflow <= 1'b0;
            end
        end
    end

    always_comb begin
        // Nothing to derive yet; the port is driven above.
    end

    assert property (@(posedge clk) disable iff (!rst_n)
        enable |-> ##1 value != $past(value));

endmodule

module top (
    input  logic clk,
    input  logic rst_n,
    input  logic enable,
    output logic [7:0] count
);

    logic overflow;

    counter #(
        .WIDTH(8),
        .LIMIT(200)
    ) u_counter (
        .clk(clk),
        .rst_n(rst_n),
        .enable(enable),
        .value(count),
        .overflow(overflow)
    );

    // Written before always_ff was habit, and never revisited.
    always @(posedge clk) begin
        if (overflow) begin
            $display("wrapped");
        end
    end

endmodule
