import {
  BlockTypeSelect,
  BoldItalicUnderlineToggles,
  CodeToggle,
  CreateLink,
  HighlightToggle,
  headingsPlugin,
  InsertImage,
  InsertTable,
  InsertThematicBreak,
  imagePlugin,
  ListsToggle,
  linkDialogPlugin,
  linkPlugin,
  listsPlugin,
  markdownShortcutPlugin,
  quotePlugin,
  Separator,
  StrikeThroughSupSubToggles,
  tablePlugin,
  thematicBreakPlugin,
  toolbarPlugin,
  UndoRedo,
} from "@mdxeditor/editor";

/** The plugins that render markdown: no toolbar, so it suits a read-only field. */
const defaultFieldPlugins = [
  headingsPlugin(),
  imagePlugin(),
  linkPlugin(),
  linkDialogPlugin(),
  listsPlugin(),
  markdownShortcutPlugin(),
  quotePlugin(),
  tablePlugin(),
  thematicBreakPlugin(),
];

/** The field plugins plus a formatting toolbar, for an editable input. */
const defaultInputPlugins = [
  ...defaultFieldPlugins,
  toolbarPlugin({
    toolbarContents: () => (
      <>
        <UndoRedo />
        <Separator />
        <BoldItalicUnderlineToggles />
        <HighlightToggle />
        <StrikeThroughSupSubToggles />
        <Separator />
        <ListsToggle />
        <Separator />
        <BlockTypeSelect />
        <Separator />
        <CreateLink />
        <InsertImage />
        <InsertTable />
        <InsertThematicBreak />
        <Separator />
        <CodeToggle />
      </>
    ),
  }),
];

export { defaultFieldPlugins, defaultInputPlugins };
