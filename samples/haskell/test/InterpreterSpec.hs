module InterpreterSpec (spec) where

import Interpreter
import Test.Hspec

spec :: Spec
spec = do
  describe "assemble" $ do
    it "reads a program it can understand" $
      assemble "push 1\npush 2\nadd" `shouldSatisfy` isRight

    it "says what it could not read, rather than guessing" $
      assemble "push 1\nwibble" `shouldSatisfy` isLeft

  describe "disassemble" $
    it "round-trips a program back to something assemble accepts" $ do
      let source = "push 7\npush 35\nadd"
      case assemble source of
        Left err -> expectationFailure err
        Right program -> assemble (disassemble program) `shouldBe` Right program

  describe "runProgram" $ do
    it "leaves the answer where the caller can read it" $
      runProgram <$> assemble "push 2\npush 3\nadd\nprint"
        `shouldSatisfy` either (const False) (either (const False) (== ["5"]))

    it "refuses to run for ever" $
      -- A jump to itself would loop; the step limit turns that into an
      -- error a caller can report rather than a process nobody can kill.
      case assemble "jmp 0" of
        Left err -> expectationFailure err
        Right program -> run program emptyMachine `shouldSatisfy` isLeft

isRight :: Either a b -> Bool
isRight = either (const False) (const True)

isLeft :: Either a b -> Bool
isLeft = either (const True) (const False)
