-- Drives the counter and checks that it wraps.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter_tb is
end entity counter_tb;

architecture sim of counter_tb is

    signal clk      : std_logic := '0';
    signal rst_n    : std_logic := '0';
    signal enable   : std_logic := '0';
    signal value    : unsigned(7 downto 0);
    signal overflow : std_logic;

begin

    u_dut : entity work.counter
        generic map (
            width => 8,
            limit => 3
        )
        port map (
            clk      => clk,
            rst_n    => rst_n,
            enable   => enable,
            value    => value,
            overflow => overflow
        );

    clocking : process
    begin
        wait for 5 ns;
        clk <= not clk;
    end process clocking;

    stimulus : process
    begin
        wait for 20 ns;
        rst_n <= '1';
        wait for 10 ns;
        enable <= '1';
        wait for 200 ns;
        assert overflow = '1' report "never wrapped" severity failure;
        wait;
    end process stimulus;

end architecture sim;
