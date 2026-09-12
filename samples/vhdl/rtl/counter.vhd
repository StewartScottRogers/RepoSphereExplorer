-- A counter, and the block that instantiates it.
--
-- The `latched` process in `top` has no sensitivity list and no wait, so
-- it runs once when simulation starts and never again. It is left that
-- way so the file pane has something to warn about.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter is
    generic (
        width : positive := 8;
        limit : natural  := 255
    );
    port (
        clk      : in  std_logic;
        rst_n    : in  std_logic;
        enable   : in  std_logic;
        value    : out unsigned(width - 1 downto 0);
        overflow : out std_logic
    );
end entity counter;

architecture rtl of counter is

    signal count : unsigned(width - 1 downto 0);

begin

    clocked : process (clk, rst_n)
    begin
        if rst_n = '0' then
            count    <= (others => '0');
            overflow <= '0';
        elsif rising_edge(clk) then
            if enable = '1' then
                if to_integer(count) = limit then
                    count    <= (others => '0');
                    overflow <= '1';
                else
                    count    <= count + 1;
                    overflow <= '0';
                end if;
            end if;
        end if;
    end process clocked;

    driving : process (count)
    begin
        value <= count;
    end process driving;

end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;

entity top is
    port (
        clk    : in  std_logic;
        rst_n  : in  std_logic;
        enable : in  std_logic;
        wrapped : out std_logic
    );
end entity top;

architecture structural of top is

    signal counted   : unsigned(7 downto 0);
    signal overflowed : std_logic;

begin

    u_counter : entity work.counter
        generic map (
            width => 8,
            limit => 200
        )
        port map (
            clk      => clk,
            rst_n    => rst_n,
            enable   => enable,
            value    => counted,
            overflow => overflowed
        );

    latched : process
    begin
        wrapped <= overflowed;
    end process latched;

end architecture structural;
