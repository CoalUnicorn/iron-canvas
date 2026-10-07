declare module "*.js" {
  export default function init(
    input?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module,
  ): Promise<unknown>;

  export class SheetTab {
    readonly index: number;
    readonly name: string;
  }

  export class CanvasRenderer {
    constructor(canvas: HTMLCanvasElement);
    renderScene(scene: unknown): void;
    resize(width: number, height: number, dpr: number): void;
  }

  export class Workbook {
    constructor(canvas: HTMLCanvasElement);
    addSheet(): void;
    getSheets(): SheetTab[];
    loadXlsx(bytes: Uint8Array, name: string): void;
    resize(width: number, height: number, dpr: number): void;
    scrollBy(rowDelta: number, columnDelta: number): void;
    selectSheet(index: number): void;
    selectedSheet(): number;
    workbookName(): string;
  }
}
