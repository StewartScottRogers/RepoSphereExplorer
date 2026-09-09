{-# LANGUAGE LambdaCase #-}
{-# LANGUAGE OverloadedStrings #-}

-- | A stack machine: an instruction set, an evaluator that threads state
-- through Either for failure, and a small assembler from text.
module Interpreter
  ( Instruction (..)
  , Machine (..)
  , MachineError (..)
  , assemble
  , run
  , runProgram
  , disassemble
  ) where

import Data.Char (isDigit, isSpace)
import Data.List (intercalate)
import qualified Data.Map.Strict as Map
import Data.Map.Strict (Map)

data Instruction
  = Push Int
  | Pop
  | Dup
  | Swap
  | Add
  | Sub
  | Mul
  | Store String
  | Load String
  | JumpIfZero Int
  | Print
  | Halt
  deriving (Eq, Show)

data Machine = Machine
  { stack       :: [Int]
  , variables   :: Map String Int
  , output      :: [String]
  , counter     :: Int
  , stepsTaken  :: Int
  } deriving (Eq, Show)

data MachineError
  = StackUnderflow Instruction
  | UnknownVariable String
  | BadJump Int
  | StepLimit Int
  deriving (Eq, Show)

emptyMachine :: Machine
emptyMachine = Machine [] Map.empty [] 0 0

stepLimit :: Int
stepLimit = 10000

binary :: Instruction -> (Int -> Int -> Int) -> Machine -> Either MachineError Machine
binary instruction op machine =
  case stack machine of
    (a : b : rest) -> Right machine { stack = op b a : rest }
    _              -> Left (StackUnderflow instruction)

step :: [Instruction] -> Machine -> Either MachineError (Maybe Machine)
step program machine
  | stepsTaken machine > stepLimit = Left (StepLimit stepLimit)
  | counter machine < 0 || counter machine >= length program = Right Nothing
  | otherwise =
      let instruction = program !! counter machine
          advanced = machine { counter = counter machine + 1
                             , stepsTaken = stepsTaken machine + 1 }
      in fmap Just $ case instruction of
           Push value -> Right advanced { stack = value : stack advanced }
           Pop -> case stack advanced of
             (_ : rest) -> Right advanced { stack = rest }
             _          -> Left (StackUnderflow instruction)
           Dup -> case stack advanced of
             (top : rest) -> Right advanced { stack = top : top : rest }
             _            -> Left (StackUnderflow instruction)
           Swap -> case stack advanced of
             (a : b : rest) -> Right advanced { stack = b : a : rest }
             _              -> Left (StackUnderflow instruction)
           Add -> binary instruction (+) advanced
           Sub -> binary instruction (-) advanced
           Mul -> binary instruction (*) advanced
           Store name -> case stack advanced of
             (top : rest) -> Right advanced { stack = rest
                                            , variables = Map.insert name top (variables advanced) }
             _ -> Left (StackUnderflow instruction)
           Load name -> case Map.lookup name (variables advanced) of
             Just value -> Right advanced { stack = value : stack advanced }
             Nothing    -> Left (UnknownVariable name)
           JumpIfZero target -> case stack advanced of
             (0 : rest)
               | target < 0 || target > length program -> Left (BadJump target)
               | otherwise -> Right advanced { stack = rest, counter = target }
             (_ : rest) -> Right advanced { stack = rest }
             _          -> Left (StackUnderflow instruction)
           Print -> case stack advanced of
             (top : rest) -> Right advanced { stack = rest
                                            , output = show top : output advanced }
             _ -> Left (StackUnderflow instruction)
           Halt -> Right advanced { counter = length program }

run :: [Instruction] -> Machine -> Either MachineError Machine
run program machine =
  step program machine >>= \case
    Nothing   -> Right machine
    Just next -> run program next

runProgram :: [Instruction] -> Either MachineError [String]
runProgram program = reverse . output <$> run program emptyMachine

assemble :: String -> Either String [Instruction]
assemble = traverse parseLine . filter (not . blank) . lines
  where
    blank line = all isSpace line || take 1 (dropWhile isSpace line) == "#"

    parseLine line = case words line of
      ["push", value] | all isDigit value -> Right (Push (read value))
      ["pop"]                             -> Right Pop
      ["dup"]                             -> Right Dup
      ["swap"]                            -> Right Swap
      ["add"]                             -> Right Add
      ["sub"]                             -> Right Sub
      ["mul"]                             -> Right Mul
      ["store", name]                     -> Right (Store name)
      ["load", name]                      -> Right (Load name)
      ["jz", target] | all isDigit target -> Right (JumpIfZero (read target))
      ["print"]                           -> Right Print
      ["halt"]                            -> Right Halt
      _                                   -> Left ("cannot assemble: " <> line)

disassemble :: [Instruction] -> String
disassemble = intercalate "\n" . zipWith render [0 :: Int ..]
  where
    render index instruction = show index <> "  " <> show instruction

main :: IO ()
main = do
  let source = unlines
        [ "# (3 + 4) * 2, printed"
        , "push 3"
        , "push 4"
        , "add"
        , "push 2"
        , "mul"
        , "dup"
        , "store answer"
        , "print"
        , "halt"
        ]
  case assemble source of
    Left problem  -> putStrLn problem
    Right program -> do
      putStrLn (disassemble program)
      case runProgram program of
        Left err     -> putStrLn ("machine error: " <> show err)
        Right result -> mapM_ putStrLn result
