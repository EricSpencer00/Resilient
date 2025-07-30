# NoLang

### A uselss programming language

ProgramNode
  └── MainMethodNode
      ├── Parameters: [VariableNode("String[]", "args")]
      └── Body: BlockNode
          ├── VariableDeclarationNode(Type: "int", Name: "myNumber", Value: LiteralNode(10))
          ├── VariableDeclarationNode(Type: "int", Name: "anotherNumber", Value: LiteralNode(5))
          ├── VariableDeclarationNode(Type: "int", Name: "sum", Value: BinaryOperationNode(Operator: "+", Left: VariableNode("myNumber"), Right: VariableNode("anotherNumber")))
          └── PrintStatementNode(Argument: VariableNode("sum"))