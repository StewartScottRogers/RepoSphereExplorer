// Drives the counter and checks it wraps where it should.
`timescale 1ns / 1ps

module counter_tb;

    logic       clk;
    logic       rst_n;
    logic       enable;
    logic [7:0] value;
    logic       overflow;

    counter #(
        .WIDTH(8),
        .LIMIT(3)
    ) u_dut (
        .clk(clk),
        .rst_n(rst_n),
        .enable(enable),
        .value(value),
        .overflow(overflow)
    );

    initial begin
        clk = 1'b0;
        forever #5 clk = ~clk;
    end

    initial begin
        rst_n  = 1'b0;
        enable = 1'b0;
        #20 rst_n  = 1'b1;
        #10 enable = 1'b1;
        #100 $finish;
    end

    always_ff @(posedge clk) begin
        if (overflow) begin
            $display("wrapped at %0t", $time);
        end
    end

endmodule
