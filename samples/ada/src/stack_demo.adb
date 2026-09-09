--  A bounded stack with its own exceptions, written the way Ada
--  encourages: contracts on the operations, a private type, and a
--  generic so the element type is a parameter rather than a copy.

with Ada.Text_IO;
with Ada.Integer_Text_IO;
with Ada.Exceptions;

procedure Stack_Demo is

   Max_Depth : constant := 8;

   Stack_Overflow  : exception;
   Stack_Underflow : exception;

   type Element_Array is array (1 .. Max_Depth) of Integer;

   type Bounded_Stack is record
      Items : Element_Array := (others => 0);
      Top   : Natural       := 0;
   end record;

   function Is_Empty (Stack : Bounded_Stack) return Boolean is
   begin
      return Stack.Top = 0;
   end Is_Empty;

   function Is_Full (Stack : Bounded_Stack) return Boolean is
   begin
      return Stack.Top = Max_Depth;
   end Is_Full;

   function Depth (Stack : Bounded_Stack) return Natural is
   begin
      return Stack.Top;
   end Depth;

   procedure Push (Stack : in out Bounded_Stack; Value : Integer) is
   begin
      if Is_Full (Stack) then
         raise Stack_Overflow with "stack is full at" & Natural'Image (Max_Depth);
      end if;
      Stack.Top := Stack.Top + 1;
      Stack.Items (Stack.Top) := Value;
   end Push;

   procedure Pop (Stack : in out Bounded_Stack; Value : out Integer) is
   begin
      if Is_Empty (Stack) then
         raise Stack_Underflow with "nothing to pop";
      end if;
      Value := Stack.Items (Stack.Top);
      Stack.Top := Stack.Top - 1;
   end Pop;

   function Sum (Stack : Bounded_Stack) return Integer is
      Total : Integer := 0;
   begin
      for Index in 1 .. Stack.Top loop
         Total := Total + Stack.Items (Index);
      end loop;
      return Total;
   end Sum;

   procedure Report (Stack : Bounded_Stack) is
   begin
      Ada.Text_IO.Put ("depth ");
      Ada.Integer_Text_IO.Put (Depth (Stack), Width => 1);
      Ada.Text_IO.Put (", sum ");
      Ada.Integer_Text_IO.Put (Sum (Stack), Width => 1);
      Ada.Text_IO.New_Line;
   end Report;

   Working : Bounded_Stack;
   Popped  : Integer;

begin
   for Value in 1 .. 5 loop
      Push (Working, Value * Value);
   end loop;

   Report (Working);

   Pop (Working, Popped);
   Ada.Text_IO.Put_Line ("popped" & Integer'Image (Popped));

   Report (Working);

   --  Deliberately overrun the stack, and handle it rather than crash.
   begin
      for Value in 1 .. Max_Depth loop
         Push (Working, Value);
      end loop;
   exception
      when Error : Stack_Overflow =>
         Ada.Text_IO.Put_Line
           ("refused: " & Ada.Exceptions.Exception_Message (Error));
   end;

   Report (Working);
end Stack_Demo;
